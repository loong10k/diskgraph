use super::ChildSpawnError;
#[cfg(unix)]
use super::UnixChild as PlatformChild;
#[cfg(windows)]
use super::WindowsTestBirth as PlatformChild;

#[test]
fn checkpoint_before_any_os_creation_preserves_a_non_clone_original_error() {
    use std::process::Command;

    #[derive(Debug)]
    struct OriginalFailure(String);
    let mut calls = 0;
    let mut command = Command::new(std::env::current_exe().unwrap());
    let error = match PlatformChild::spawn(&mut command, || {
        calls += 1;
        Err::<(), _>(OriginalFailure("original cancellation".into()))
    }) {
        Ok(_) => panic!("original checkpoint must reject before OS creation"),
        Err(error) => error,
    };
    assert_eq!(calls, 1);
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None }
        if primary.0 == "original cancellation")
    );
}

#[cfg(unix)]
#[test]
fn generic_non_clone_checkpoint_failure_runs_after_real_child_creation_and_cleanup() {
    use super::UnixChild;
    use std::process::Command;

    #[derive(Debug)]
    struct OriginalFailure(String);
    let mut checks = 0;
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "sleep 20"]);
    let error = match UnixChild::spawn(&mut command, || {
        checks += 1;
        if checks == 2 {
            Err(OriginalFailure("original authority denied".into()))
        } else {
            Ok(())
        }
    }) {
        Ok(_) => panic!("second original checkpoint must reject the child"),
        Err(error) => error,
    };
    assert_eq!(checks, 2);
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None }
        if primary.0 == "original authority denied")
    );
}
