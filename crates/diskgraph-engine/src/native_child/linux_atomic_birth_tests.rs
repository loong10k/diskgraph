//! PF06 原子出生的独立 Linux 机制资格；C 成功不充 Rust 生产路径已修复。

use super::linux_atomic_birth_fixture::AtomicBirthFixture;
use super::linux_atomic_birth_test_support as fixture;

#[test]
fn atomic_birth_original_pidfd_waits_external_kill_before_first_ready() {
    let report = fixture::run(&AtomicBirthFixture::new(), "kill_before_ready");
    assert_eq!(report["kernel_cloexec"], true);
    assert_eq!(report["original_reaped"], true);
    assert_eq!(report["exit_status"], libc::SIGKILL);
    assert_eq!(report["output_bytes"], 0);
}

#[test]
fn atomic_birth_original_pidfd_waits_natural_death_before_first_ready() {
    let report = fixture::run(&AtomicBirthFixture::new(), "natural");
    assert_eq!(report["kernel_cloexec"], true);
    assert_eq!(report["original_reaped"], true);
    assert_eq!(report["exit_status"], 37);
    assert_eq!(report["output_bytes"], 0);
}

#[test]
fn atomic_birth_consumed_identity_is_echild_and_other_child_stays_alive() {
    let report = fixture::run(&AtomicBirthFixture::new(), "external_reap");
    assert_eq!(report["original_reaped"], true);
    assert_eq!(report["external_reap_observed"], true);
    assert_eq!(report["other_alive"], true);
    assert_eq!(
        report["pid_reuse_verified"], false,
        "real reuse remains unverified"
    );
}

#[test]
fn atomic_birth_multithread_child_does_not_run_host_handler_or_atfork() {
    let report = fixture::run(&AtomicBirthFixture::new(), "signals");
    assert_eq!(report["thread_count"], 2);
    assert_eq!(report["exit_status"], libc::SIGUSR1);
    assert_eq!(report["output_bytes"], 0);
    assert_eq!(report["original_reaped"], true);
    assert_eq!(report["parent_handler_preserved"], true);
}

#[test]
fn atomic_birth_held_elf_executes_original_image_after_real_path_replacement() {
    let report = fixture::run(&AtomicBirthFixture::new(), "held_image");
    assert_eq!(report["original_reaped"], true);
    assert_eq!(report["exit_status"], 0);
    assert_eq!(
        report["output_bytes"],
        b"{\"image\":1,\"canary_closed\":true}\n".len()
    );
    assert_eq!(report["ambient_closed"], true);
}

#[test]
fn atomic_birth_close_range_denial_preserves_original_errno_and_no_exec() {
    let report = fixture::run(&AtomicBirthFixture::new(), "close_range_denied");
    assert_eq!(report["actual_errno"], libc::EACCES);
    assert_eq!(report["exit_status"], 127);
    assert_eq!(report["original_reaped"], true);
}

#[test]
fn atomic_birth_denied_legacy_birth_preserves_errno_and_spawns_no_child() {
    let report = fixture::run(&AtomicBirthFixture::new(), "clone_denied");
    assert_eq!(report["actual_errno"], libc::EACCES);
    assert_eq!(
        report["original_reaped"], false,
        "kernel created no child to reap"
    );
    assert_eq!(report["kernel_cloexec"], false);
}

#[test]
fn atomic_birth_real_clone3_enosys_does_not_prevent_legacy_atomic_birth() {
    let report = fixture::run(&AtomicBirthFixture::new(), "clone3_denied");
    assert_eq!(report["actual_errno"], libc::ENOSYS);
    assert_eq!(report["kernel_cloexec"], true);
    assert_eq!(report["original_reaped"], true);
}
