//! Promoting a metadata comparison to a content one.
//!
//! The whole point of this pass is the row metadata got wrong: two files of
//! identical length, edited in place. Before it, they compare the same. After
//! it, they do not. The tests below are mostly about the other half - what
//! happens when the read cannot happen, because "I could not check" must never
//! arrive as "they match".

use std::sync::Arc;

use diskgraph_core::{Evidence, PrincipalId, ScopeId, Verdict};
use diskgraph_engine::verify::{VerifyBudget, verify_same_rows};
use diskgraph_engine::{Engine, EngineConfig};

struct Fixture {
    _workspace: tempfile::TempDir,
    _left: tempfile::TempDir,
    _right: tempfile::TempDir,
    engine: Arc<Engine>,
    left: String,
    right: String,
    left_scope: ScopeId,
    right_scope: ScopeId,
}

fn fixture() -> Fixture {
    let workspace = tempfile::TempDir::with_prefix("dg-verify-").unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 100_000,
            scan_budget: diskgraph_core::ScanBudget {
                max_nodes: 100_000,
                max_staging_bytes: 1 << 30,
                ..diskgraph_core::ScanBudget::default()
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let left = tempfile::TempDir::with_prefix("dg-verify-l-").unwrap();
    let right = tempfile::TempDir::with_prefix("dg-verify-r-").unwrap();

    // Same length, different bytes: the case metadata cannot see.
    let mut divergent = vec![0x5A_u8; 8_192];
    divergent[0] = 0x01;
    let mut other = divergent.clone();
    other[0] = 0x02;
    std::fs::write(left.path().join("divergent.bin"), &divergent).unwrap();
    std::fs::write(right.path().join("divergent.bin"), &other).unwrap();
    // Genuinely identical.
    std::fs::write(left.path().join("identical.bin"), vec![0x33_u8; 4_096]).unwrap();
    std::fs::write(right.path().join("identical.bin"), vec![0x33_u8; 4_096]).unwrap();

    let principal = PrincipalId::new("verify").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let index = |root: &std::path::Path| {
        let scope = engine
            .register_scope(root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        // register_scope granted scope-local rights into the policy store;
        // the authorizer taken before it is a snapshot that does not have
        // them yet, and asking for the job with it is refused.
        let authorizer = engine.policy_authorizer().unwrap();
        let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
        engine.run_job(&job.job_id, "verify").unwrap();
        (
            scope.clone(),
            engine.latest_revision(&scope).unwrap().unwrap(),
        )
    };
    let (left_scope, left_revision) = index(left.path());
    let (right_scope, right_revision) = index(right.path());
    Fixture {
        _workspace: workspace,
        _left: left,
        _right: right,
        engine,
        left: left_revision,
        right: right_revision,
        left_scope,
        right_scope,
    }
}

fn compare(fixture: &Fixture) -> diskgraph_engine::ComparisonReport {
    fixture
        .engine
        .compare_revisions(&fixture.left, &fixture.right, 4)
        .unwrap()
}

fn verify(
    fixture: &Fixture,
    report: diskgraph_engine::ComparisonReport,
    budget: VerifyBudget,
) -> (
    diskgraph_engine::ComparisonReport,
    diskgraph_engine::verify::VerifySummary,
) {
    let authorizer = fixture.engine.policy_authorizer().unwrap();
    verify_same_rows(
        &fixture.engine,
        report,
        &fixture.left_scope,
        &fixture.right_scope,
        &PrincipalId::new("verify").unwrap(),
        &authorizer,
        budget,
    )
    .unwrap()
}

#[test]
fn metadata_alone_calls_two_different_files_the_same() {
    // The premise of this whole module. If this ever stops being true the
    // comparison got stronger and the verification pass has less to add than
    // it claims.
    let fixture = fixture();
    let report = compare(&fixture);
    let row = report
        .rows
        .iter()
        .find(|row| row.path == "divergent.bin")
        .expect("row present");
    assert!(
        !row.verdict.is_difference(),
        "same length and a recent timestamp is all metadata sees"
    );
}

#[test]
fn reading_contents_separates_the_two_files_metadata_confused() {
    let fixture = fixture();
    let authorizer = fixture.engine.policy_authorizer().unwrap();
    let principal = PrincipalId::new("verify").unwrap();
    // Reading contents is a grant nobody gets by registering a scope.
    fixture
        .engine
        .set_content_read(&fixture.left_scope, &principal, true)
        .unwrap();
    fixture
        .engine
        .set_content_read(&fixture.right_scope, &principal, true)
        .unwrap();
    let _ = authorizer;
    let (report, summary) = verify(&fixture, compare(&fixture), VerifyBudget::default());
    let row = report
        .rows
        .iter()
        .find(|row| row.path == "divergent.bin")
        .expect("row present");
    assert_eq!(
        row.verdict,
        Verdict::Different {
            reason: diskgraph_core::DifferentReason::Content
        },
        "eight kilobytes apart at byte zero, once read"
    );
    assert_eq!(summary.confirmed_different, 1);
    assert_eq!(summary.confirmed_same, 1);
    assert_eq!(summary.unverified, 0);
    assert!(summary.is_complete());
    assert!(summary.bytes_read > 0, "and it says how much it cost");
}

#[test]
fn an_ungranted_read_is_unverified_and_never_identical() {
    let fixture = fixture();
    // No content grant: the pass runs, reads nothing, and says so.
    let (report, summary) = verify(&fixture, compare(&fixture), VerifyBudget::default());
    assert_eq!(summary.confirmed_same, 0);
    assert_eq!(summary.confirmed_different, 0);
    assert_eq!(summary.unverified, 2, "both rows were left unsaid");
    assert!(!summary.is_complete());
    assert_eq!(summary.bytes_read, 0);
    // The metadata verdict survives, because it was never wrong - it was just
    // weaker than the caller wanted.
    for row in &report.rows {
        assert!(matches!(row.verdict, Verdict::Same { .. }));
    }
}

#[test]
fn a_file_over_the_budget_is_left_unverified_rather_than_hashed() {
    let fixture = fixture();
    let principal = PrincipalId::new("verify").unwrap();
    fixture
        .engine
        .set_content_read(&fixture.left_scope, &principal, true)
        .unwrap();
    fixture
        .engine
        .set_content_read(&fixture.right_scope, &principal, true)
        .unwrap();
    let budget = VerifyBudget {
        max_files: 10,
        // Everything in the fixture is above this.
        max_bytes_per_file: 16,
    };
    let (_, summary) = verify(&fixture, compare(&fixture), budget);
    assert_eq!(summary.bytes_read, 0, "nothing was read");
    assert_eq!(summary.unverified, 2);
    assert!(!summary.is_complete());
}

#[test]
fn the_file_budget_bounds_the_pass() {
    let fixture = fixture();
    let principal = PrincipalId::new("verify").unwrap();
    fixture
        .engine
        .set_content_read(&fixture.left_scope, &principal, true)
        .unwrap();
    fixture
        .engine
        .set_content_read(&fixture.right_scope, &principal, true)
        .unwrap();
    let budget = VerifyBudget {
        max_files: 1,
        max_bytes_per_file: 1 << 20,
    };
    let (_, summary) = verify(&fixture, compare(&fixture), budget);
    assert!(
        summary.confirmed_same + summary.confirmed_different <= 1,
        "the caller said one file: {}",
        summary.confirmed_same + summary.confirmed_different
    );
    assert!(!summary.is_complete());
}

#[test]
fn withdrawing_the_grant_stops_the_reads() {
    let fixture = fixture();
    let principal = PrincipalId::new("verify").unwrap();
    for scope in [&fixture.left_scope, &fixture.right_scope] {
        fixture
            .engine
            .set_content_read(scope, &principal, true)
            .unwrap();
    }
    let (_, granted) = verify(&fixture, compare(&fixture), VerifyBudget::default());
    assert!(granted.bytes_read > 0, "the grant worked");

    for scope in [&fixture.left_scope, &fixture.right_scope] {
        fixture
            .engine
            .set_content_read(scope, &principal, false)
            .unwrap();
    }
    let (_, withdrawn) = verify(&fixture, compare(&fixture), VerifyBudget::default());
    assert_eq!(withdrawn.bytes_read, 0, "and taking it back worked");
    assert_eq!(withdrawn.unverified, 2);
}

#[test]
fn a_verified_row_says_it_was_read_not_inferred() {
    let fixture = fixture();
    let principal = PrincipalId::new("verify").unwrap();
    for scope in [&fixture.left_scope, &fixture.right_scope] {
        fixture
            .engine
            .set_content_read(scope, &principal, true)
            .unwrap();
    }
    let (report, _) = verify(&fixture, compare(&fixture), VerifyBudget::default());
    let identical = report
        .rows
        .iter()
        .find(|row| row.path == "identical.bin")
        .expect("row present");
    assert_eq!(
        identical.verdict,
        Verdict::Same {
            evidence: Evidence::Content
        },
        "a caller that skips a copy on this row is relying on bytes that \
         were actually read"
    );
}
