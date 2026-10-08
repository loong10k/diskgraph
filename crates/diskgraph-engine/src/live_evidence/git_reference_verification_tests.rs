//! 同一采样的纯引用语法结论可复用，HEAD身份和存在性必须重新读取。
use super::git_references::{head, verify_head};
use super::probe_output::ProbeOutput;

fn reply(args: &[&str], oid: Option<&str>, branch: Option<&str>) -> ProbeOutput {
    let (stdout, code) = match args {
        ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"] => match oid {
            Some(value) => (format!("{value}\n").into_bytes(), 0),
            None => (Vec::new(), 1),
        },
        ["symbolic-ref", "--quiet", "--no-recurse", "HEAD"] => match branch {
            Some(value) => (format!("{value}\n").into_bytes(), 0),
            None => (Vec::new(), 1),
        },
        ["check-ref-format", _] => (Vec::new(), 0),
        ["show-ref", "--exists", _] => (Vec::new(), 2),
        _ => panic!("unexpected native reference command: {args:?}"),
    };
    ProbeOutput {
        stdout,
        stderr: Vec::new(),
        exit_code: Some(code),
    }
}

#[test]
fn same_verified_branch_reuses_only_syntax_and_reads_both_identity_fields_again() {
    for width in [40, 64] {
        let oid = "a".repeat(width);
        let mut commands = Vec::new();
        let mut run = |args: &[&str]| {
            commands.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            Ok(reply(args, Some(&oid), Some("refs/heads/main")))
        };
        let observed = head(&mut run).unwrap();
        verify_head(&mut run, &observed).unwrap();
        assert_eq!(
            commands
                .iter()
                .filter(|a| a[0] == "check-ref-format")
                .count(),
            1
        );
        assert_eq!(commands.iter().filter(|a| a[0] == "rev-parse").count(), 2);
        assert_eq!(
            commands.iter().filter(|a| a[0] == "symbolic-ref").count(),
            2
        );
    }
}

#[test]
fn different_branch_is_validated_and_rejected_while_changed_oid_is_not_cached() {
    for (terminal_oid, terminal_branch) in [
        ("a".repeat(40), "refs/heads/other"),
        ("b".repeat(40), "refs/heads/main"),
    ] {
        let observed =
            head(&mut |a| Ok(reply(a, Some(&"a".repeat(40)), Some("refs/heads/main")))).unwrap();
        let mut validated = Vec::new();
        let error = verify_head(
            &mut |args| {
                if args[0] == "check-ref-format" {
                    validated.push(args[1].to_string());
                }
                Ok(reply(args, Some(&terminal_oid), Some(terminal_branch)))
            },
            &observed,
        )
        .unwrap_err();
        assert_eq!(error, "HEAD changed during Git sampling");
        if terminal_branch != "refs/heads/main" {
            assert_eq!(validated, [terminal_branch]);
        } else {
            assert!(validated.is_empty());
        }
    }
}

#[test]
fn fresh_observations_do_not_share_validation_and_unborn_conflicts_are_rechecked() {
    let mut validations = 0;
    let mut run = |args: &[&str]| {
        if args[0] == "check-ref-format" {
            validations += 1;
        }
        Ok(reply(args, None, Some("refs/heads/main")))
    };
    let first = head(&mut run).unwrap();
    head(&mut run).unwrap();
    assert_eq!(
        validations, 2,
        "separate observations cannot share syntax state"
    );
    let error = verify_head(
        &mut |args| {
            let mut result = reply(args, None, Some("refs/heads/main"));
            if args[0] == "show-ref" {
                result.exit_code = Some(0);
            }
            Ok(result)
        },
        &first,
    )
    .unwrap_err();
    assert_eq!(
        error,
        "HEAD reference exists but does not resolve to a commit"
    );
}

#[test]
fn unborn_checks_reference_existence_twice_and_detached_reads_head_twice() {
    for (oid, branch) in [
        (None, Some("refs/heads/main")),
        (Some("a".repeat(40)), None),
    ] {
        let mut commands = Vec::new();
        let mut run = |args: &[&str]| {
            commands.push(args[0].to_string());
            Ok(reply(args, oid.as_deref(), branch))
        };
        let observed = head(&mut run).unwrap();
        verify_head(&mut run, &observed).unwrap();
        assert_eq!(commands.iter().filter(|a| *a == "rev-parse").count(), 2);
        assert_eq!(commands.iter().filter(|a| *a == "symbolic-ref").count(), 2);
        assert_eq!(
            commands.iter().filter(|a| *a == "show-ref").count(),
            if oid.is_none() { 2 } else { 0 }
        );
    }
}

#[test]
fn terminal_native_failure_and_invalid_new_branch_keep_original_errors() {
    let observed =
        head(&mut |a| Ok(reply(a, Some(&"a".repeat(40)), Some("refs/heads/main")))).unwrap();
    assert_eq!(
        verify_head(&mut |_| Err("original deadline".into()), &observed).unwrap_err(),
        "original deadline"
    );
    let error = verify_head(
        &mut |args| {
            if args[0] == "check-ref-format" {
                return Err("original invalid reference".into());
            }
            Ok(reply(
                args,
                Some(&"a".repeat(40)),
                Some("refs/heads/bad..name"),
            ))
        },
        &observed,
    )
    .unwrap_err();
    assert_eq!(error, "original invalid reference");
}
