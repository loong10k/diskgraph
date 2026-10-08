//! CLI 查询必须继承 dispatch 请求的绝对期限，而不是在数据阶段重新计时。
use crate::cli_engine_host::CliTestEngine;
use clap::Parser;
use diskgraph_core::{BusinessError, PrincipalId, QueryBudget};
use diskgraph_engine::{EngineConfig, EngineError};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

thread_local! {
    static BEFORE_REPLY: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// 编码 hook 武装一次真实授权等待，避免靠首次解析速度碰巧超时。
struct ArmedAuthorizer {
    delegate: diskgraph_core::PolicyAuthorizer,
    armed: Arc<AtomicBool>,
    delayed: Arc<AtomicBool>,
    deadline: Instant,
}

impl diskgraph_core::Authorizer for ArmedAuthorizer {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &diskgraph_core::Permission,
        scope: &diskgraph_core::ScopeId,
    ) -> diskgraph_core::Decision {
        if self.armed.swap(false, Ordering::SeqCst) {
            std::thread::sleep(
                self.deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5),
            );
            self.delayed.store(true, Ordering::SeqCst);
        }
        self.delegate.decide(principal, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.delegate.policy_version()
    }
}

fn terminal_delayed_comparison(plan: bool) -> (Result<(), EngineError>, Vec<String>) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"data").unwrap();
    let engine = CliTestEngine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "late-reply-fixture").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    std::fs::write(root.join("extra"), b"second revision").unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "late-reply-fixture-2").unwrap();
    let other_revision = engine.latest_revision(&scope).unwrap().unwrap();
    assert_ne!(revision, other_revision);
    let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
    let armed = Arc::new(AtomicBool::new(false));
    let delayed = Arc::new(AtomicBool::new(false));
    let authorizer = ArmedAuthorizer {
        delegate: engine.policy_authorizer().unwrap(),
        armed: Arc::clone(&armed),
        delayed: Arc::clone(&delayed),
        deadline,
    };
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            armed.store(true, Ordering::SeqCst);
        }))
    });
    let mut args = vec![
        "diskgraph",
        "compare",
        "--from",
        &revision,
        "--to",
        &other_revision,
    ];
    if plan {
        args.push("--plan");
    }
    let cli = super::Cli::try_parse_from(args).unwrap();
    let mut output = Vec::new();
    let result = super::dispatch(
        &engine,
        &cli,
        &principal,
        &authorizer,
        &mut output,
        deadline,
    );
    assert!(
        delayed.load(Ordering::SeqCst),
        "the encoded terminal phase was not exercised"
    );
    (result, output)
}

#[test]
fn plan_cannot_escape_as_usable_steps_when_encoding_terminal_authorization_expires() {
    let (result, output) = terminal_delayed_comparison(true);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(
                BusinessError::BudgetExceeded | BusinessError::Timeout
            ))
        ) || matches!(result, Err(EngineError::Store(ref error)) if error.is_interrupted() || matches!(error, diskgraph_store::StoreError::BudgetExceeded)),
        "late plan remained usable: {result:?}, {output:?}"
    );
    assert!(output.is_empty());
}

#[test]
fn comparison_summary_is_partial_when_encoded_terminal_authorization_expires() {
    let (result, output) = terminal_delayed_comparison(false);
    if result.is_ok() {
        let reply: serde_json::Value = serde_json::from_str(&output[0]).unwrap();
        assert_eq!(reply["data"]["complete"], false);
        assert_eq!(reply["truncated"], true);
        assert_eq!(reply["data"]["summary_is_partial"], true);
    } else {
        assert!(
            matches!(
                result,
                Err(EngineError::Business(
                    BusinessError::BudgetExceeded | BusinessError::Timeout
                ))
            ) || matches!(result, Err(EngineError::Store(ref error)) if error.is_interrupted() || matches!(error, diskgraph_store::StoreError::BudgetExceeded)),
            "{result:?}"
        );
        assert!(output.is_empty());
    }
}

pub(super) fn before_reply() {
    if let Some(hook) = BEFORE_REPLY.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[test]
fn tree_and_history_do_not_complete_after_the_dispatch_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"data").unwrap();
    let engine = CliTestEngine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let authorizer = engine.policy_authorizer().unwrap();
    let scope = engine
        .register_scope(&root, &principal, &authorizer)
        .unwrap();
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "fixture").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let cases = [
        vec!["tree", "--scope", scope.as_str()],
        vec!["compare", "--from", &revision, "--to", &revision],
        vec!["compare", "--from", &revision, "--to", &revision, "--plan"],
        vec![
            "changes",
            "--scope",
            scope.as_str(),
            "--before",
            &revision,
            "--after",
            &revision,
        ],
        vec![
            "growth",
            "--scope",
            scope.as_str(),
            "--before",
            &revision,
            "--after",
            &revision,
        ],
    ];
    let mut failures = Vec::new();
    for args in cases {
        let label = args[0];
        let cli = super::Cli::try_parse_from(std::iter::once("diskgraph").chain(args)).unwrap();
        let mut output = Vec::new();
        let result = super::dispatch(
            &engine,
            &cli,
            &principal,
            &authorizer,
            &mut output,
            Instant::now() - Duration::from_millis(QueryBudget::default().deadline_ms),
        );
        if let Ok(()) = result {
            let reply: serde_json::Value = serde_json::from_str(&output[0]).unwrap();
            if reply["data"]["complete"] != false || reply["truncated"] != true {
                failures.push(format!("{label} ignored the dispatch deadline: {reply}"));
            }
        } else {
            assert!(
                matches!(
                    &result,
                    Err(EngineError::Business(
                        BusinessError::BudgetExceeded | BusinessError::Timeout
                    )) | Err(EngineError::Store(
                        diskgraph_store::StoreError::BudgetExceeded
                    ))
                ) || matches!(&result, Err(EngineError::Store(error)) if error.is_interrupted()),
                "{label}: {result:?}"
            );
            assert!(output.is_empty());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn html_export_does_not_write_after_terminal_scope_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"data").unwrap();
    let data = directory.path().join("data");
    let destination = directory.path().join("report.html");
    let engine = CliTestEngine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "fixture").unwrap();
    let revoked = scope.clone();
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            diskgraph_store::ControlStore::open(&data.join("diskgraph-control.sqlite"))
                .unwrap()
                .revoke_scope(&revoked)
                .unwrap();
        }))
    });
    let cli = super::Cli::try_parse_from([
        "diskgraph",
        "tree",
        "--scope",
        scope.as_str(),
        "--html",
        destination.to_str().unwrap(),
    ])
    .unwrap();
    let mut output = Vec::new();
    let result = super::dispatch(
        &engine,
        &cli,
        &principal,
        &engine.policy_authorizer().unwrap(),
        &mut output,
        diskgraph_core::query_deadline(QueryBudget::default()).unwrap(),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "{result:?}"
    );
    assert!(!destination.exists());
    assert!(output.is_empty());
}

fn assert_encoded_query_revoked(command: &str, revoke_scope: bool, restore_grant: bool) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"data").unwrap();
    let data = directory.path().join("data");
    let engine = CliTestEngine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "fixture").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let revoked = scope.clone();
    let revoked_principal = principal.clone();
    let encoded = Arc::new(AtomicBool::new(false));
    let reached = Arc::clone(&encoded);
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            let mut control =
                diskgraph_store::ControlStore::open(&data.join("diskgraph-control.sqlite"))
                    .unwrap();
            if revoke_scope {
                control.revoke_scope(&revoked).unwrap();
            } else {
                control
                    .revoke_grant(
                        &revoked_principal,
                        &diskgraph_core::Permission::MetadataRead,
                        &revoked,
                    )
                    .unwrap();
                if restore_grant {
                    let policy_version = control.policy_version().unwrap();
                    control
                        .upsert_grant(&diskgraph_core::Grant {
                            principal: revoked_principal,
                            permission: diskgraph_core::Permission::MetadataRead,
                            scope: revoked,
                            policy_version,
                        })
                        .unwrap();
                }
            }
            reached.store(true, Ordering::SeqCst);
        }))
    });
    let html_destination = directory.path().join("withdrawal.html");
    let html_path = html_destination.to_string_lossy();
    let args = match command {
        "tree" => vec![command, "--scope", scope.as_str()],
        "html" => vec!["tree", "--scope", scope.as_str(), "--html", &html_path],
        "compare" => vec![command, "--from", &revision, "--to", &revision],
        "verified_compare" => vec![
            "compare",
            "--from-scope",
            scope.as_str(),
            "--to-scope",
            scope.as_str(),
            "--verify-content",
        ],
        "plan" => vec!["compare", "--from", &revision, "--to", &revision, "--plan"],
        _ => vec![
            command,
            "--scope",
            scope.as_str(),
            "--before",
            &revision,
            "--after",
            &revision,
        ],
    };
    let cli = super::Cli::try_parse_from(std::iter::once("diskgraph").chain(args)).unwrap();
    let mut output = Vec::new();
    let result = super::dispatch(
        &engine,
        &cli,
        &principal,
        &engine.policy_authorizer().unwrap(),
        &mut output,
        diskgraph_core::query_deadline(QueryBudget::default()).unwrap(),
    );
    assert!(
        encoded.load(Ordering::SeqCst),
        "{command}: actual encoding hook was not reached: {result:?}"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "{command}: {result:?}"
    );
    assert!(output.is_empty());
    assert!(!html_destination.exists());
}

#[test]
fn encoded_tree_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("tree", false, true);
}

#[test]
fn encoded_html_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("html", false, true);
}

#[test]
fn encoded_comparison_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("compare", false, true);
}

#[test]
fn encoded_verified_comparison_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("verified_compare", false, true);
}

#[test]
fn encoded_changes_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("changes", false, true);
}

#[test]
fn encoded_growth_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("growth", false, true);
}

#[test]
fn encoded_plan_remembers_grant_revocation_even_if_restored() {
    assert_encoded_query_revoked("plan", false, true);
}

#[test]
fn encoded_tree_refuses_terminal_scope_revocation() {
    assert_encoded_query_revoked("tree", true, false);
}
#[test]
fn encoded_comparison_refuses_terminal_grant_revocation() {
    assert_encoded_query_revoked("compare", false, false);
}
#[test]
fn encoded_changes_refuses_terminal_scope_revocation() {
    assert_encoded_query_revoked("changes", true, false);
}
#[test]
fn encoded_growth_refuses_terminal_grant_revocation() {
    assert_encoded_query_revoked("growth", false, false);
}
