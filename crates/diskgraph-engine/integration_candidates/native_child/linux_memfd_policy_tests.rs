//! 三个真实独占 namespace 策略目标；exit77 是缺证失败，不作为跳过或验收通过。

use super::linux_memfd_policy_support::{in_namespace, original_deadline};
use super::linux_scan_image_execution_fixture::{
    MFD_EXEC, ScanImageExecutionFixture, assert_seals, raw_memfd,
};
use crate::EngineError;

#[test]
fn policy_zero_executes_sealed_a_after_path_replacement() {
    in_namespace(
        0,
        "native_child::linux_memfd_policy_tests::policy_zero_executes_sealed_a_after_path_replacement",
        positive,
    );
}

#[test]
fn policy_one_executes_sealed_a_after_path_replacement() {
    in_namespace(
        1,
        "native_child::linux_memfd_policy_tests::policy_one_executes_sealed_a_after_path_replacement",
        positive,
    );
}

#[test]
fn policy_two_preserves_native_execution_creation_denial() {
    in_namespace(
        2,
        "native_child::linux_memfd_policy_tests::policy_two_preserves_native_execution_creation_denial",
        || {
            let fixture = ScanImageExecutionFixture::new(original_deadline());
            let error = raw_memfd(MFD_EXEC).expect_err("actual policy 2 must reject explicit EXEC");
            assert_eq!(error.raw_os_error(), Some(libc::EACCES));
            match fixture.prepared() {
                Err(EngineError::Io(error)) => assert_eq!(error.raw_os_error(), Some(libc::EACCES)),
                Err(other) => panic!("must preserve native policy errno, got {other:?}"),
                Ok(_) => panic!(
                    "policy 2 accepted production image: missing explicit EXEC, NX is not a substitute"
                ),
            }
        },
    );
}

fn positive() {
    let fixture = ScanImageExecutionFixture::new(original_deadline());
    let reference = fixture
        .native_image(MFD_EXEC)
        .expect("actual explicit EXEC capability");
    fixture.execute(reference, "1");
    let image = fixture.prepared().unwrap().into_file();
    assert_seals(&image);
    fixture.replace_with_verified_b();
    fixture.execute(image, "1");
}
