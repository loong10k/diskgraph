//! scoped 源正文实际读取的精确额度回归；来源：D35 有界捕获，不模拟 read 返回值。
use super::read;
use crate::live_evidence::ProbeLimits;
use crate::live_evidence::git_metadata_budget::GitMetadataBudget;
use crate::live_evidence::probe_budget::ProbeBudget;
use std::fs::File;
use std::io::Seek;

#[test]
fn real_source_reads_admit_empty_and_exact_length_without_lookahead() {
    for length in [0, 1, 4096, 8193] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source");
        let bytes = vec![5u8; length];
        std::fs::write(&path, &bytes).unwrap();
        let mut file = File::open(&path).unwrap();
        let mut budget = GitMetadataBudget::new(length, 8).unwrap();
        let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
        let mut captured = Vec::new();
        read(&mut file, length as u64, &mut budget, &mut probe, |chunk| {
            captured.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        assert_eq!(captured, bytes);
        assert_eq!(budget.remaining_bytes(), 0);
        assert_eq!(file.stream_position().unwrap(), length as u64);
    }
}

#[test]
fn over_budget_body_is_not_read_even_when_native_file_is_long_enough() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"12345").unwrap();
    let mut file = File::open(&path).unwrap();
    let mut budget = GitMetadataBudget::new(4, 8).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = read(&mut file, 5, &mut budget, &mut probe, |_| {
        panic!("inadmissible source bytes escaped")
    })
    .unwrap_err();
    assert!(error.contains("byte limit"));
    assert_eq!(budget.remaining_bytes(), 4);
    assert_eq!(file.stream_position().unwrap(), 0);
}

#[test]
fn growth_after_length_observation_reads_no_extra_byte_and_refuses_result() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"1234").unwrap();
    let mut file = File::open(&path).unwrap();
    let observed = file.metadata().unwrap().len();
    std::fs::write(&path, b"12345678").unwrap();
    let mut budget = GitMetadataBudget::new(4, 8).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut captured = Vec::new();
    let error = read(&mut file, observed, &mut budget, &mut probe, |chunk| {
        captured.extend_from_slice(chunk);
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("length changed"));
    assert_eq!(captured, b"1234");
    assert_eq!(budget.remaining_bytes(), 0);
    assert_eq!(file.stream_position().unwrap(), 4);
}

#[test]
fn shortening_after_length_observation_charges_only_real_read_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"12345678").unwrap();
    let mut file = File::open(&path).unwrap();
    let observed = file.metadata().unwrap().len();
    std::fs::write(&path, b"123").unwrap();
    let mut budget = GitMetadataBudget::new(8, 8).unwrap();
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let error = read(&mut file, observed, &mut budget, &mut probe, |_| Ok(())).unwrap_err();
    assert!(error.contains("shortened"));
    assert_eq!(budget.remaining_bytes(), 5);
    assert_eq!(file.stream_position().unwrap(), 3);
}

#[cfg(unix)]
#[test]
fn initial_version_uses_the_same_metadata_observation_as_copied_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"body").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let file = File::open(&path).unwrap();
    let observed = file.metadata().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let current = file.metadata().unwrap();
    let version = super::initial_version(&file, &observed).unwrap();
    assert_eq!(observed.permissions().mode() & 0o777, 0o644);
    assert_eq!(current.permissions().mode() & 0o777, 0o755);
    assert!(version.matches_metadata(&observed));
    assert!(
        !version.matches_metadata(&current),
        "later chmod must remain a detected version change"
    );
}
