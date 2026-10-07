//! EC-04 的真实私有对象库回归；仅使用临时 Git 仓库和实际子进程。

use super::ProbeLimits;
use super::git_isolation_fixture::GitIsolationFixture;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_output::successful;
use super::git_view::GitView;
#[cfg(windows)]
use super::native_probe_test_budget::NativeProbeTestBudget as ProbeBudget;
#[cfg(not(windows))]
use super::probe_budget::ProbeBudget;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn git_head(fixture: &GitIsolationFixture) -> String {
    String::from_utf8(fixture.git(&["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

fn assert_unsupported(result: Result<super::GitSample, String>, reason: &str) {
    let error = result.expect_err("unsupported source ODB was borrowed by Git");
    assert!(error.contains(reason), "{error}");
}

fn object_watermark(fixture: &GitIsolationFixture) -> Vec<(PathBuf, Vec<u8>, Vec<u64>)> {
    let root = fixture.path().join(".git/objects");
    let mut pending = vec![root.clone()];
    let mut result = Vec::new();
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        }
        let bytes = if metadata.is_file() {
            Sha256::digest(std::fs::read(&path).unwrap()).to_vec()
        } else {
            Vec::new()
        };
        #[cfg(unix)]
        let version = {
            use std::os::unix::fs::MetadataExt;
            vec![
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
                metadata.nlink(),
                metadata.mtime() as u64,
                metadata.mtime_nsec() as u64,
                metadata.ctime() as u64,
                metadata.ctime_nsec() as u64,
            ]
        };
        #[cfg(windows)]
        let version = {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_STANDARD_INFO,
                FileBasicInfo, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx,
            };
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&path)
                .unwrap();
            let mut id = FILE_ID_INFO::default();
            let mut basic = FILE_BASIC_INFO::default();
            let mut standard = FILE_STANDARD_INFO::default();
            assert_ne!(
                unsafe {
                    GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileIdInfo,
                        (&mut id as *mut FILE_ID_INFO).cast(),
                        std::mem::size_of::<FILE_ID_INFO>() as u32,
                    )
                },
                0
            );
            assert_ne!(
                unsafe {
                    GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileBasicInfo,
                        (&mut basic as *mut FILE_BASIC_INFO).cast(),
                        std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                    )
                },
                0
            );
            assert_ne!(
                unsafe {
                    GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileStandardInfo,
                        (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                        std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
                    )
                },
                0
            );
            let mut values = vec![
                id.VolumeSerialNumber,
                standard.EndOfFile as u64,
                basic.CreationTime as u64,
                basic.LastWriteTime as u64,
                basic.ChangeTime as u64,
                standard.NumberOfLinks as u64,
            ];
            values.extend(id.FileId.Identifier.iter().map(|byte| u64::from(*byte)));
            values
        };
        result.push((path.strip_prefix(&root).unwrap().to_owned(), bytes, version));
    }
    result.sort();
    result
}

#[test]
fn source_alternates_and_recursive_foreign_objects_are_rejected() {
    for recursive in [false, true] {
        let a = GitIsolationFixture::new("sha1");
        let b = GitIsolationFixture::new("sha1");
        let c = GitIsolationFixture::new("sha1");
        b.git(&["commit", "--allow-empty", "-q", "-m", "foreign B"]);
        c.git(&["commit", "--allow-empty", "-q", "-m", "foreign C"]);
        let outside = if recursive { &c } else { &b };
        let head = git_head(outside);
        std::fs::write(a.path().join(".git/refs/heads/main"), format!("{head}\n")).unwrap();
        let b_objects = super::git_native_path::alternate(&b.path().join(".git/objects")).unwrap();
        std::fs::write(a.path().join(".git/objects/info/alternates"), b_objects).unwrap();
        if recursive {
            std::fs::write(
                b.path().join(".git/objects/info/alternates"),
                super::git_native_path::alternate(&c.path().join(".git/objects")).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            a.git(&["rev-parse", "HEAD^{commit}"]),
            format!("{head}\n").as_bytes()
        );
        let before = object_watermark(&a);
        let result = a.sample();
        assert_eq!(object_watermark(&a), before);
        assert_unsupported(result, "unsupported Git object alternates");
    }
}

#[test]
fn oversized_alternates_are_rejected_without_parsing_the_source_file() {
    let fixture = GitIsolationFixture::new("sha1");
    let path = fixture.path().join(".git/objects/info/alternates");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len((65 << 20) + 1).unwrap();
    drop(file);
    assert_unsupported(fixture.sample(), "unsupported Git object alternates");
}

#[test]
fn unreferenced_loose_objects_cannot_escape_the_shared_byte_budget() {
    let fixture = GitIsolationFixture::new("sha1");
    let fanout = fixture.path().join(".git/objects/ff");
    std::fs::create_dir_all(&fanout).unwrap();
    let file = std::fs::File::create(fanout.join("1".repeat(38))).unwrap();
    file.set_len((65 << 20) + 1).unwrap();
    drop(file);
    let error = fixture
        .sample()
        .expect_err("source loose object was not counted");
    assert!(
        error.contains("git metadata byte limit exceeded"),
        "{error}"
    );
}

#[test]
fn late_alternate_is_not_a_git_input_and_source_change_is_rejected() {
    let fixture = GitIsolationFixture::new("sha1");
    let foreign = GitIsolationFixture::new("sha1");
    foreign.git(&["commit", "--allow-empty", "-q", "-m", "foreign object"]);
    let foreign_oid = git_head(&foreign);
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = GitView::prepare(
        Path::new("git"),
        fixture.path(),
        &mut probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let private = PathBuf::from(
        String::from_utf8(
            successful(
                view.run(&["rev-parse", "--absolute-git-dir"], &mut probe)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .trim(),
    );
    std::fs::write(
        fixture.path().join(".git/objects/info/alternates"),
        super::git_native_path::alternate(&foreign.path().join(".git/objects")).unwrap(),
    )
    .unwrap();
    let foreign_lookup = view
        .run(
            &["cat-file", "-e", &format!("{foreign_oid}^{{commit}}")],
            &mut probe,
        )
        .unwrap();
    let verified = view.verify(&mut probe);
    let finished = view.complete(verified);
    assert!(
        !private.exists(),
        "private data survived explicit completion"
    );
    assert_ne!(
        foreign_lookup.exit_code,
        Some(0),
        "late alternate resolved a foreign object"
    );
    assert!(
        finished
            .unwrap_err()
            .contains("git metadata directory changed")
    );
}

#[cfg(unix)]
#[test]
fn loose_links_are_rejected_before_git_can_open_a_foreign_file() {
    let fixture = GitIsolationFixture::new("sha1");
    let target = fixture.path().parent().unwrap().join("foreign-file");
    std::fs::write(&target, b"foreign").unwrap();
    let fanout = fixture.path().join(".git/objects/ff");
    std::fs::create_dir_all(&fanout).unwrap();
    std::os::unix::fs::symlink(&target, fanout.join("2".repeat(38))).unwrap();
    assert!(fixture.sample().is_err(), "source ODB link was borrowed");
}

#[test]
fn unknown_loose_names_are_not_normalized_or_ignored() {
    let fixture = GitIsolationFixture::new("sha1");
    let fanout = fixture.path().join(".git/objects/ff");
    std::fs::create_dir_all(&fanout).unwrap();
    std::fs::write(fanout.join("UNKNOWN"), b"not an object").unwrap();
    assert_unsupported(fixture.sample(), "unsupported Git loose object name");
}

#[test]
fn captured_loose_content_changes_are_detected_at_the_terminal_guard() {
    let fixture = GitIsolationFixture::new("sha1");
    let oid = String::from_utf8(fixture.git(&["hash-object", "-w", "tracked"])).unwrap();
    let oid = oid.trim();
    let source = fixture
        .path()
        .join(".git/objects")
        .join(&oid[..2])
        .join(&oid[2..]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&source).unwrap().permissions().mode();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(mode | 0o200)).unwrap();
    }
    #[cfg(windows)]
    {
        let mut permissions = std::fs::metadata(&source).unwrap().permissions();
        // Windows 仅清除临时对象的 FILE_ATTRIBUTE_READONLY；Unix 已单独只补 owner 写位。
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "仅 Windows 测试夹具调整只读属性，不修改 Unix 权限或 Windows ACL"
        )]
        permissions.set_readonly(false);
        std::fs::set_permissions(&source, permissions).unwrap();
    }
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = GitView::prepare(
        Path::new("git"),
        fixture.path(),
        &mut probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let mut bytes = std::fs::read(&source).unwrap();
    bytes[0] ^= 1;
    std::fs::write(source, bytes).unwrap();
    let verified = view.verify(&mut probe);
    let finished = view.complete(verified);
    assert!(
        finished.is_err(),
        "source object change was not part of the terminal guard"
    );
}

#[test]
fn packed_sha1_and_sha256_keep_status_stash_and_upstream_without_source_writes() {
    for format in ["sha1", "sha256"] {
        let fixture = GitIsolationFixture::new(format);
        std::fs::write(fixture.path().join("tracked"), b"stash\n").unwrap();
        fixture.git(&["stash", "push", "-q"]);
        fixture.git(&["config", "remote.origin.url", "."]);
        fixture.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ]);
        fixture.git(&["config", "branch.main.remote", "origin"]);
        fixture.git(&["config", "branch.main.merge", "refs/heads/main"]);
        fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        fixture.git(&["commit", "--allow-empty", "-q", "-m", "ahead"]);
        fixture.git(&["repack", "-a", "-d"]);
        for entry in std::fs::read_dir(fixture.path().join(".git/objects")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name().len() == 2 {
                assert!(
                    std::fs::read_dir(entry.path()).unwrap().next().is_none(),
                    "pack-only fixture retained a loose object"
                );
            }
        }
        assert!(
            std::fs::read_dir(fixture.path().join(".git/objects/pack"))
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "pack"))
        );
        std::fs::write(fixture.path().join("tracked"), b"working\n").unwrap();
        let before = object_watermark(&fixture);
        let sample = fixture.sample().unwrap();
        assert_eq!(sample.dirty_count, 1);
        assert_eq!(sample.stash_count, 1);
        assert_eq!(sample.ahead_of_upstream, Some(1));
        assert_eq!(sample.behind_upstream, Some(0));
        assert_eq!(object_watermark(&fixture), before);
    }
}

#[test]
fn orphan_pack_and_promisor_markers_are_explicitly_rejected() {
    for (extension, reason) in [
        ("pack", "unsupported Git unpaired pack/index"),
        ("idx", "unsupported Git unpaired pack/index"),
        ("promisor", "unsupported Git promisor object store"),
    ] {
        let fixture = GitIsolationFixture::new("sha1");
        let name = format!("pack-{}.{}", "0".repeat(40), extension);
        std::fs::write(
            fixture.path().join(".git/objects/pack").join(name),
            b"marker",
        )
        .unwrap();
        assert_unsupported(fixture.sample(), reason);
    }
}

#[test]
fn known_object_accelerators_are_not_private_git_inputs() {
    let fixture = GitIsolationFixture::new("sha1");
    let objects = fixture.path().join(".git/objects");
    let sidecars = [
        PathBuf::from("info/commit-graph"),
        PathBuf::from("info/packs"),
        PathBuf::from("pack/multi-pack-index"),
        PathBuf::from(format!("pack/pack-{}.bitmap", "0".repeat(40))),
    ];
    for path in &sidecars {
        std::fs::write(objects.join(path), b"not a valid accelerator").unwrap();
    }
    std::fs::create_dir(objects.join("info/commit-graphs")).unwrap();
    std::fs::write(objects.join("info/commit-graphs/unused"), b"not copied").unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut view = GitView::prepare(
        Path::new("git"),
        fixture.path(),
        &mut probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
    .unwrap();
    let private = PathBuf::from(
        String::from_utf8(
            successful(
                view.run(&["rev-parse", "--absolute-git-dir"], &mut probe)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .trim(),
    );
    for path in &sidecars {
        assert!(!private.join("objects").join(path).exists());
    }
    assert!(!private.join("objects/info/commit-graphs").exists());
    let status = successful(
        view.run(&["status", "--porcelain=v1", "-z"], &mut probe)
            .unwrap(),
    );
    assert!(status.unwrap().is_empty());
    let verified = view.verify(&mut probe);
    view.complete(verified).unwrap();
    assert!(!private.exists());
}

#[cfg(unix)]
#[test]
fn ignored_object_sidecars_do_not_hide_links() {
    for relative in [
        "info/commit-graph",
        "pack/multi-pack-index",
        "info/commit-graphs",
    ] {
        let fixture = GitIsolationFixture::new("sha1");
        let target = fixture.path().parent().unwrap().join("foreign-sidecar");
        std::fs::write(&target, b"outside").unwrap();
        std::os::unix::fs::symlink(&target, fixture.path().join(".git/objects").join(relative))
            .unwrap();
        assert_unsupported(fixture.sample(), "unsupported Git object sidecar type");
    }
}

#[cfg(unix)]
#[test]
fn ignored_sidecar_type_queries_do_not_follow_a_replaced_parent() {
    use super::git_metadata_tree::GitMetadataTree;
    use super::git_private_directory::GitPrivateDirectory;
    let fixture = GitIsolationFixture::new("sha1");
    let objects = fixture.path().join(".git/objects");
    let info = objects.join("info");
    std::fs::write(info.join("commit-graph"), b"original ignored metadata").unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut budget = GitMetadataBudget::default();
    let mut metadata = GitMetadataTree::default();
    assert!(metadata.directory(&info, &mut budget, &mut probe).unwrap());
    let outside = tempfile::tempdir().unwrap();
    let outside = outside.path().canonicalize().unwrap();
    std::fs::write(outside.join("commit-graph"), b"foreign ignored metadata").unwrap();
    std::fs::rename(&info, fixture.path().join("old-info")).unwrap();
    std::os::unix::fs::symlink(&outside, &info).unwrap();
    let mut private = GitPrivateDirectory::new(&mut probe).unwrap();
    let target = private.path().join("objects");
    private.create_dir_all(&target, &mut probe).unwrap();
    let result = super::git_object_database::copy_flat(
        &objects,
        &target,
        20,
        &mut metadata,
        &mut private,
        &mut budget,
        &mut probe,
    );
    assert!(
        result.is_err(),
        "ignored type-only query followed a replaced parent"
    );
    private.complete(Ok(())).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn native_non_utf8_loose_names_are_not_lossily_normalized() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = GitIsolationFixture::new("sha1");
    let fanout = fixture.path().join(".git/objects/ff");
    // 初始提交的对象哈希可能已使用 ff 分桶；保留既有对象，再注入非法原生名称。
    std::fs::create_dir_all(&fanout).unwrap();
    let name = std::ffi::OsString::from_vec(vec![0xff; 38]);
    std::fs::write(fanout.join(name), b"invalid native name").unwrap();
    assert_unsupported(fixture.sample(), "unsupported Git loose object name");
}
