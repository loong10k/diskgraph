//! 实际 ELF fd-exec 验收：没有路径回退、没有把密封成功当执行成功。

use super::linux_memfd_policy_support::{original_deadline, policy};
use super::linux_scan_image_execution_fixture::{
    MFD_EXEC, MFD_NOEXEC_SEAL, ScanImageExecutionFixture, assert_seals,
};

#[test]
fn sealed_actual_elf_keeps_a_after_path_replacement_with_verified_b() {
    let end = original_deadline();
    let scope = policy();
    assert!(
        scope <= 1,
        "missing_qualification: current policy={scope} cannot prove positive execution"
    );
    let fixture = ScanImageExecutionFixture::new(end);
    let reference = fixture.native_image(MFD_EXEC).unwrap_or_else(|error| {
        panic!(
            "missing_qualification phase=explicit_exec_memfd errno={:?} error={error}",
            error.raw_os_error()
        )
    });
    fixture.execute(reference, "1");
    let prepared = fixture.prepared().unwrap().into_file();
    assert_seals(&prepared);
    fixture.replace_with_verified_b();
    fixture.execute(prepared, "1");
    eprintln!(
        "MEMFD_EXEC_CURRENT qualified=true policy={scope} image_a=true image_b=true original_wait=true"
    );
}

#[test]
fn four_sealed_nx_elf_is_rejected_by_real_execveat() {
    let fixture = ScanImageExecutionFixture::new(original_deadline());
    let image = fixture
        .native_image(MFD_NOEXEC_SEAL)
        .unwrap_or_else(|error| {
            panic!(
                "missing_qualification phase=noexec_memfd errno={:?} error={error}",
                error.raw_os_error()
            )
        });
    fixture.reject_nx(image);
}
