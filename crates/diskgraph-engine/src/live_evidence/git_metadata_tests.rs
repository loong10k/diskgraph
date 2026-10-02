use super::git_index_layout::GitIndexLayout;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_file::GitMetadataFile;
use super::probe_budget::ProbeBudget;
use super::probe_limits::ProbeLimits;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;

fn probe() -> ProbeBudget {
    ProbeBudget::new(&ProbeLimits {
        max_output_bytes: 0,
        ..ProbeLimits::default()
    })
    .unwrap()
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
}

fn seal_index(bytes: &mut [u8], oid_len: usize) {
    let end = bytes.len() - oid_len;
    let checksum = if oid_len == 20 {
        Sha1::digest(&bytes[..end]).to_vec()
    } else {
        Sha256::digest(&bytes[..end]).to_vec()
    };
    bytes[end..].copy_from_slice(&checksum);
}

#[test]
fn metadata_budget_is_independent_of_probe_pipe_output() {
    let mut probe = probe();
    let mut budget = GitMetadataBudget::new(4, 1).unwrap();
    budget.charge_entry(&mut probe).unwrap();
    budget.charge_bytes(4, &mut probe).unwrap();
    assert!(budget.charge_bytes(1, &mut probe).is_err());
    assert!(budget.charge_entry(&mut probe).is_err());
}

#[test]
fn native_file_capture_and_verify_share_the_byte_budget() {
    let temp = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap().join("index");
    std::fs::write(&path, b"four").unwrap();
    let mut probe = probe();
    let mut budget = GitMetadataBudget::new(7, 2).unwrap();
    let captured = GitMetadataFile::capture(&path, &mut budget, &mut probe).unwrap();
    assert_eq!(captured.bytes(), Some(b"four".as_slice()));
    assert!(captured.modified().is_some());
    assert!(captured.verify(&mut budget, &mut probe).is_err());
    let mut budget = GitMetadataBudget::new(8, 2).unwrap();
    let captured = GitMetadataFile::capture(&path, &mut budget, &mut probe).unwrap();
    captured.verify(&mut budget, &mut probe).unwrap();
}

#[test]
fn file_change_and_missing_parent_do_not_verify_as_stable() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let path = root.join("index");
    std::fs::write(&path, b"first").unwrap();
    let mut probe = probe();
    let mut budget = GitMetadataBudget::default();
    let captured = GitMetadataFile::capture(&path, &mut budget, &mut probe).unwrap();
    std::fs::write(&path, b"later").unwrap();
    assert!(captured.verify(&mut budget, &mut probe).is_err());
    assert!(GitMetadataFile::capture(&root.join("missing/leaf"), &mut budget, &mut probe).is_err());
    #[cfg(unix)]
    {
        let absent =
            GitMetadataFile::capture(&root.join("absent"), &mut budget, &mut probe).unwrap();
        assert_eq!(absent.bytes(), None);
        absent.verify(&mut budget, &mut probe).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn links_and_fifo_are_refused_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    std::fs::write(root.join("real"), b"data").unwrap();
    std::os::unix::fs::symlink("real", root.join("link")).unwrap();
    std::os::unix::fs::symlink(&root, root.join("parent-link")).unwrap();
    let fifo = root.join("fifo");
    let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    let mut probe = probe();
    let mut budget = GitMetadataBudget::default();
    for path in [root.join("link"), root.join("parent-link/real"), fifo] {
        assert!(
            GitMetadataFile::capture(&path, &mut budget, &mut probe).is_err(),
            "{path:?}"
        );
    }
}

#[test]
fn metadata_capture_observes_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap().join("index");
    std::fs::write(&path, b"data").unwrap();
    let limits = ProbeLimits::default();
    let mut probe = ProbeBudget::new(&limits).unwrap();
    limits.cancel.store(true, Ordering::Release);
    let mut budget = GitMetadataBudget::default();
    assert!(GitMetadataFile::capture(&path, &mut budget, &mut probe).is_err());
}

#[test]
fn git_index_v2_v4_and_compatible_v3_layout_are_preflighted() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    std::fs::write(temp.path().join("alpha"), b"one").unwrap();
    std::fs::write(temp.path().join("beta"), b"two").unwrap();
    git(temp.path(), &["add", "alpha", "beta"]);
    for version in [2, 4] {
        git(
            temp.path(),
            &["update-index", "--index-version", &version.to_string()],
        );
        let index = std::fs::read(temp.path().join(".git/index")).unwrap();
        let layout =
            GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe())
                .unwrap();
        assert_eq!(layout.version, version);
        assert_eq!(layout.entry_count, 2);
        if version == 2 {
            // 无扩展条目时 Git 可将请求的 v3 写回 v2；由真实 v2 条目组成兼容 v3 结构。
            let mut compatible_v3 = index.clone();
            compatible_v3[4..8].copy_from_slice(&3u32.to_be_bytes());
            seal_index(&mut compatible_v3, 20);
            let layout = GitIndexLayout::parse(
                &compatible_v3,
                20,
                &mut GitMetadataBudget::default(),
                &mut probe(),
            )
            .unwrap();
            assert_eq!(layout.version, 3);
        }
    }
}

#[test]
fn unsupported_index_mode_and_extension_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    std::fs::write(temp.path().join("alpha"), b"one").unwrap();
    git(temp.path(), &["add", "alpha"]);
    let index = std::fs::read(temp.path().join(".git/index")).unwrap();
    let mut gitlink = index.clone();
    gitlink[12 + 24..12 + 28].copy_from_slice(&0o160000u32.to_be_bytes());
    seal_index(&mut gitlink, 20);
    assert!(
        GitIndexLayout::parse(
            &gitlink,
            20,
            &mut GitMetadataBudget::default(),
            &mut probe()
        )
        .is_err()
    );
    let mut split = index.clone();
    let at = split.len() - 20;
    split.splice(at..at, [b'l', b'i', b'n', b'k', 0, 0, 0, 0]);
    seal_index(&mut split, 20);
    assert!(
        GitIndexLayout::parse(&split, 20, &mut GitMetadataBudget::default(), &mut probe()).is_err()
    );
}

#[test]
fn sha256_git_index_uses_full_oid_width() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q", "--object-format=sha256"]);
    std::fs::write(temp.path().join("alpha"), b"one").unwrap();
    git(temp.path(), &["add", "alpha"]);
    let index = std::fs::read(temp.path().join(".git/index")).unwrap();
    let layout =
        GitIndexLayout::parse(&index, 32, &mut GitMetadataBudget::default(), &mut probe()).unwrap();
    assert_eq!(layout.entry_count, 1);
    assert!(
        GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe()).is_err()
    );
}

#[test]
fn index_checksum_corruption_is_rejected_for_sha1_and_sha256() {
    for (format, oid_len) in [("sha1", 20usize), ("sha256", 32usize)] {
        let temp = tempfile::tempdir().unwrap();
        git(
            temp.path(),
            &["init", "-q", &format!("--object-format={format}")],
        );
        std::fs::write(temp.path().join("alpha"), b"one").unwrap();
        git(temp.path(), &["add", "alpha"]);
        let mut index = std::fs::read(temp.path().join(".git/index")).unwrap();
        GitIndexLayout::parse(
            &index,
            oid_len,
            &mut GitMetadataBudget::default(),
            &mut probe(),
        )
        .unwrap();
        let last = index.len() - 1;
        index[last] ^= 1;
        assert!(
            GitIndexLayout::parse(
                &index,
                oid_len,
                &mut GitMetadataBudget::default(),
                &mut probe(),
            )
            .is_err(),
            "{format} index checksum was accepted"
        );
    }
}

#[test]
fn private_git_view_rejects_checksum_corruption_before_status() {
    for format in ["sha1", "sha256"] {
        let temp = tempfile::tempdir().unwrap();
        git(
            temp.path(),
            &["init", "-q", &format!("--object-format={format}")],
        );
        std::fs::write(temp.path().join("alpha"), b"one").unwrap();
        git(temp.path(), &["add", "alpha"]);
        let path = temp.path().join(".git/index");
        let mut index = std::fs::read(&path).unwrap();
        let last = index.len() - 1;
        index[last] ^= 1;
        std::fs::write(path, index).unwrap();
        let error =
            super::sample_git_bounded(Path::new("git"), temp.path(), &ProbeLimits::default())
                .unwrap_err();
        assert!(error.contains("checksum"), "{format}: {error}");
    }
}

#[test]
fn assume_valid_bit_preserves_git_status_semantics_in_private_view() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    std::fs::write(temp.path().join("alpha"), b"one").unwrap();
    git(temp.path(), &["add", "alpha"]);
    git(
        temp.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=f@example.test",
            "commit",
            "-q",
            "-m",
            "first",
        ],
    );
    git(
        temp.path(),
        &["update-index", "--assume-unchanged", "alpha"],
    );
    let index = std::fs::read(temp.path().join(".git/index")).unwrap();
    let flags = u16::from_be_bytes(index[12 + 60..12 + 62].try_into().unwrap());
    assert_ne!(flags & 0x8000, 0, "Git did not set assume-valid");
    std::fs::write(temp.path().join("alpha"), b"changed working tree").unwrap();
    let original = Command::new("git")
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(original.status.success(), "{original:?}");
    assert!(original.stdout.is_empty(), "{original:?}");
    let layout =
        GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe()).unwrap();
    assert_eq!(layout.entry_count, 1);
    let sample =
        super::sample_git_bounded(Path::new("git"), temp.path(), &ProbeLimits::default()).unwrap();
    assert_eq!(sample.dirty_count, 0);
    assert_eq!(
        std::fs::read(temp.path().join(".git/index")).unwrap(),
        index
    );
}

#[test]
fn extended_flags_reserved_bit_is_rejected_with_valid_checksum() {
    let mut index = Vec::new();
    index.extend_from_slice(b"DIRC");
    index.extend_from_slice(&3u32.to_be_bytes());
    index.extend_from_slice(&1u32.to_be_bytes());
    let mut entry = [0u8; 62];
    entry[24..28].copy_from_slice(&0o100644u32.to_be_bytes());
    entry[60..62].copy_from_slice(&0x4005u16.to_be_bytes());
    index.extend_from_slice(&entry);
    index.extend_from_slice(&0x8000u16.to_be_bytes());
    index.extend_from_slice(b"alpha\0");
    while (index.len() - 12) % 8 != 0 {
        index.push(0);
    }
    index.extend_from_slice(&[0u8; 20]);
    seal_index(&mut index, 20);
    assert!(
        GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe()).is_err()
    );
}

#[test]
fn index_rejects_git_metadata_path_with_valid_checksum() {
    let mut index = Vec::new();
    index.extend_from_slice(b"DIRC");
    index.extend_from_slice(&2u32.to_be_bytes());
    index.extend_from_slice(&1u32.to_be_bytes());
    let mut entry = [0u8; 62];
    entry[24..28].copy_from_slice(&0o100644u32.to_be_bytes());
    entry[60..62].copy_from_slice(&11u16.to_be_bytes());
    index.extend_from_slice(&entry);
    index.extend_from_slice(b".git/config\0");
    while (index.len() - 12) % 8 != 0 {
        index.push(0);
    }
    index.extend_from_slice(&[0u8; 20]);
    seal_index(&mut index, 20);
    assert!(
        GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe()).is_err()
    );
}

#[test]
fn index_tracks_more_entries_than_metadata_file_limit() {
    let count = 200_000u32;
    let mut index = Vec::with_capacity(20_000_000);
    index.extend_from_slice(b"DIRC");
    index.extend_from_slice(&2u32.to_be_bytes());
    index.extend_from_slice(&count.to_be_bytes());
    for number in 0..count {
        let start = index.len();
        let mut entry = [0u8; 62];
        entry[24..28].copy_from_slice(&0o100644u32.to_be_bytes());
        let path = format!("file-{number:06}");
        entry[60..62].copy_from_slice(&(path.len() as u16).to_be_bytes());
        index.extend_from_slice(&entry);
        index.extend_from_slice(path.as_bytes());
        index.push(0);
        while (index.len() - start) % 8 != 0 {
            index.push(0);
        }
    }
    index.extend_from_slice(&[0u8; 20]);
    seal_index(&mut index, 20);
    let mut budget = GitMetadataBudget::new(32 << 20, 1).unwrap();
    let layout = GitIndexLayout::parse(&index, 20, &mut budget, &mut probe()).unwrap();
    assert_eq!(layout.entry_count, count);
}

#[test]
fn index_v4_varint_prefix_over_127_bytes_is_bounded() {
    let mut index = Vec::new();
    index.extend_from_slice(b"DIRC");
    index.extend_from_slice(&4u32.to_be_bytes());
    index.extend_from_slice(&2u32.to_be_bytes());
    for (path_len, strip, suffix) in [
        (130u16, &[0u8][..], vec![b'a'; 130]),
        (1u16, &[0x80u8, 0x02][..], vec![b'b']),
    ] {
        let mut entry = [0u8; 62];
        entry[24..28].copy_from_slice(&0o100644u32.to_be_bytes());
        entry[60..62].copy_from_slice(&path_len.to_be_bytes());
        index.extend_from_slice(&entry);
        index.extend_from_slice(strip);
        index.extend_from_slice(&suffix);
        index.push(0);
    }
    index.extend_from_slice(&[0u8; 20]);
    seal_index(&mut index, 20);
    let layout =
        GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe()).unwrap();
    assert_eq!(layout.entry_count, 2);
    let at = 12 + 62 + 1 + 130 + 1 + 62;
    index[at] = 0xff;
    seal_index(&mut index, 20);
    assert!(
        GitIndexLayout::parse(&index, 20, &mut GitMetadataBudget::default(), &mut probe()).is_err()
    );
}

#[cfg(unix)]
#[test]
fn same_bytes_with_changed_native_version_are_rejected() {
    use std::fs::{File, FileTimes, OpenOptions};
    use std::time::Duration;
    let temp = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap().join("index");
    std::fs::write(&path, b"same").unwrap();
    let mut budget = GitMetadataBudget::default();
    let mut probe = probe();
    let captured = GitMetadataFile::capture(&path, &mut budget, &mut probe).unwrap();
    std::fs::write(&path, b"same").unwrap();
    let file: File = OpenOptions::new().write(true).open(&path).unwrap();
    let changed = captured.modified().unwrap() - Duration::from_secs(2);
    file.set_times(FileTimes::new().set_modified(changed))
        .unwrap();
    assert!(captured.verify(&mut budget, &mut probe).is_err());
}
