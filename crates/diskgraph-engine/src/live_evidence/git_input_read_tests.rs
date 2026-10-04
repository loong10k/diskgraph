//! 普通文件输入的真实句柄偏移证明读取前准入，不能以最终报错掩盖已读超额字节。

use super::ProbeLimits;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_file::read_chunks as read_metadata;
use super::git_reflog_file::read_chunks as read_reflog;
use super::probe_budget::ProbeBudget;
use std::io::{Seek, Write};
use std::sync::atomic::Ordering;

fn input(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    file.rewind().unwrap();
    file
}

fn probe(bytes: usize) -> ProbeBudget {
    ProbeBudget::new(&ProbeLimits {
        max_output_bytes: bytes,
        ..ProbeLimits::default()
    })
    .unwrap()
}

#[test]
fn metadata_growth_never_reads_beyond_the_initial_length_or_remaining_bytes() {
    let mut file = input(&vec![b'x'; 8192]);
    let mut budget = GitMetadataBudget::new(4, 1).unwrap();
    // 已观察的长度为 4，随后文件变大；真正读取量由同一句柄的偏移证明。
    assert!(read_metadata(file.as_file_mut(), 4, &mut budget, &mut probe(0)).is_err());
    assert!(file.stream_position().unwrap() <= 4);
}

#[test]
fn metadata_zero_length_growth_does_not_read_body() {
    let mut file = input(b"later");
    let mut budget = GitMetadataBudget::new(0, 1).unwrap();
    assert!(read_metadata(file.as_file_mut(), 0, &mut budget, &mut probe(0)).is_err());
    assert_eq!(file.stream_position().unwrap(), 0);
}

#[test]
fn metadata_known_over_limit_is_refused_before_reading() {
    let mut file = input(b"12345");
    let mut budget = GitMetadataBudget::new(4, 1).unwrap();
    assert!(read_metadata(file.as_file_mut(), 5, &mut budget, &mut probe(0)).is_err());
    assert_eq!(file.stream_position().unwrap(), 0);
    assert_eq!(budget.remaining_bytes(), 4);
}

#[test]
fn metadata_exact_limit_empty_and_shrunk_files_keep_complete_semantics() {
    for bytes in [b"".as_slice(), b"four".as_slice()] {
        let mut file = input(bytes);
        let mut budget = GitMetadataBudget::new(bytes.len(), 1).unwrap();
        assert_eq!(
            read_metadata(
                file.as_file_mut(),
                bytes.len() as u64,
                &mut budget,
                &mut probe(0)
            )
            .unwrap(),
            bytes
        );
        assert_eq!(budget.remaining_bytes(), 0);
    }
    let mut file = input(b"four");
    let mut budget = GitMetadataBudget::new(8, 1).unwrap();
    assert!(read_metadata(file.as_file_mut(), 8, &mut budget, &mut probe(0)).is_err());
    assert_eq!(file.stream_position().unwrap(), 4);
    assert_eq!(budget.remaining_bytes(), 4);
}

#[test]
fn reflog_known_over_limit_is_refused_before_reading_any_body() {
    let mut file = input(&vec![b'x'; 8192]);
    let mut budget = probe(4);
    assert!(read_reflog(file.as_file_mut(), &mut budget).is_err());
    assert_eq!(file.stream_position().unwrap(), 0);
    assert_eq!(budget.remaining_bytes(), 4);
}

#[test]
fn reflog_exact_limit_and_empty_file_use_only_actual_input() {
    for bytes in [b"".as_slice(), b"four".as_slice()] {
        let mut file = input(bytes);
        let mut budget = probe(bytes.len());
        assert_eq!(read_reflog(file.as_file_mut(), &mut budget).unwrap(), bytes);
        assert_eq!(budget.remaining_bytes(), 0);
    }
}

#[test]
fn reflog_preflight_failure_cannot_become_success_on_a_later_empty_input() {
    let mut file = input(b"too large");
    let mut budget = probe(4);
    let first = read_reflog(file.as_file_mut(), &mut budget).unwrap_err();
    assert!(first.contains("output"));
    let mut empty = input(b"");
    assert_eq!(
        read_reflog(empty.as_file_mut(), &mut budget).unwrap_err(),
        first
    );
    assert_eq!(file.stream_position().unwrap(), 0);
    assert_eq!(empty.stream_position().unwrap(), 0);
    assert_eq!(budget.remaining_bytes(), 4);
}

#[test]
fn cancelled_native_input_does_not_read_or_charge() {
    let limits = ProbeLimits::default();
    let mut execution = ProbeBudget::new(&limits).unwrap();
    limits.cancel.store(true, Ordering::Release);
    let mut file = input(b"four");
    let mut budget = GitMetadataBudget::new(4, 1).unwrap();
    assert!(read_metadata(file.as_file_mut(), 4, &mut budget, &mut execution).is_err());
    assert!(read_reflog(file.as_file_mut(), &mut execution).is_err());
    assert_eq!(file.stream_position().unwrap(), 0);
    assert_eq!(budget.remaining_bytes(), 4);
}
