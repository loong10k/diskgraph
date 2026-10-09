//! 空配置解析的子进程数量、原预算与配置语义回归。

use super::ProbeLimits;
use super::git_configuration::GitConfiguration;
use super::probe_budget::ProbeBudget;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[test]
fn captured_empty_configuration_never_starts_a_parser() {
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut config = GitConfiguration::default();
    let parsed = config.parse_captured(b"", &mut budget, |_| {
        Err("empty configuration unexpectedly started a native parser".into())
    });
    assert_eq!(parsed, Ok(Vec::new()));
    assert_eq!(config.oid_len().unwrap(), 20);
}

#[test]
fn native_git_empty_file_matches_the_captured_fast_path() {
    let fixture = super::git_isolation_fixture::GitIsolationFixture::new("sha1");
    let path = fixture.path().join("empty-config-control");
    std::fs::write(&path, b"").unwrap();
    let tool_path = super::git_tool_path::from_native(&path).unwrap();
    let output = fixture.git(&[
        "config",
        "--file",
        tool_path.to_str().unwrap(),
        "--no-includes",
        "--null",
        "--list",
    ]);
    assert!(output.is_empty());
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let parsed = GitConfiguration::default()
        .parse_captured(&std::fs::read(&path).unwrap(), &mut budget, |_| {
            panic!("native positive control already proved the empty result")
        })
        .unwrap();
    assert_eq!(parsed, output);
}

#[test]
fn whitespace_and_nonempty_configuration_keep_native_parsing_and_layer_order() {
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut config = GitConfiguration::default();
    config.extend(b"core.fscache\ntrue\0").unwrap();
    let mut calls = 0;
    for captured in [b" \n".as_slice(), b"[core]\nfscache=false\n"] {
        config
            .parse_captured(captured, &mut budget, |_| {
                calls += 1;
                Ok(b"core.fscache\nfalse\0".to_vec())
            })
            .unwrap();
    }
    assert_eq!(calls, 2);
    let rendered = String::from_utf8(config.render(b"/attrs", b"/excludes").unwrap()).unwrap();
    assert_eq!(rendered.matches("fscache").count(), 3);
    assert!(rendered.find("true").unwrap() < rendered.find("false").unwrap());
}

#[test]
fn empty_configuration_cannot_bypass_original_expiry_or_cancellation() {
    for cancelled in [false, true] {
        let mut limits = ProbeLimits::default();
        if cancelled {
            limits.cancel.store(true, Ordering::Release);
        } else {
            limits.timeout = Duration::ZERO;
        }
        let mut budget = ProbeBudget::new(&limits).unwrap();
        let result = GitConfiguration::default().parse_captured(b"", &mut budget, |_| {
            panic!("invalid original budget must prevent parser entry")
        });
        assert!(result.is_err());
        assert!(budget.failure().is_some());
    }
}

#[test]
fn native_failure_and_unsupported_configuration_are_not_empty_success() {
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut config = GitConfiguration::default();
    assert_eq!(
        config.parse_captured(b"broken", &mut budget, |_| Err("native failure".into())),
        Err("native failure".into())
    );
    assert!(
        config
            .parse_captured(b"[include]", &mut budget, |_| Ok(
                b"include.path\nunsafe\0".to_vec()
            ))
            .unwrap_err()
            .contains("unsupported Git configuration include")
    );
}

#[test]
fn cancellation_during_native_parsing_prevents_merging_returned_fields() {
    let limits = ProbeLimits::default();
    let mut budget = ProbeBudget::new(&limits).unwrap();
    let mut config = GitConfiguration::default();
    let result = config.parse_captured(b"nonempty", &mut budget, |_| {
        limits.cancel.store(true, Ordering::Release);
        Ok(b"extensions.objectformat\nsha256\0".to_vec())
    });
    assert!(result.is_err());
    assert_eq!(config.oid_len().unwrap(), 20);
}
