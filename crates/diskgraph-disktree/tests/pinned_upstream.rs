//! Upstream pin and dependency-surface regression (P0 task 1.8, specs
//! AI-01 / RE-04 / RE-05). Moving the disktree pin or widening the dependency
//! surface must fail here first.

use std::path::PathBuf;

fn workspace_manifest() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../Cargo.toml")
        .canonicalize()
        .unwrap();
    std::fs::read_to_string(path).unwrap()
}

/// The reviewed upstream revision. Bumping it requires the path/permission/
/// link/placeholder/size-semantics regression pass from technical design §1.
const PINNED_REVISION: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

#[test]
fn disktree_core_is_pinned_to_the_reviewed_revision() {
    let manifest = workspace_manifest();
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("disktree-core ="))
        .expect("workspace must declare disktree-core");
    assert!(
        line.contains(&format!("rev = \"{PINNED_REVISION}\"")),
        "disktree-core must stay pinned to {PINNED_REVISION}, found: {line}"
    );
    assert!(
        line.contains("git = \"https://github.com/tobi/disktree\""),
        "disktree-core must come from the pinned upstream repository"
    );
}

#[test]
fn no_upstream_app_or_product_dependency_leaks_into_the_workspace() {
    let manifest = workspace_manifest();
    for forbidden in ["disktree-app", "prunex", "agentscope"] {
        assert!(
            !manifest.to_ascii_lowercase().contains(forbidden),
            "workspace must not depend on {forbidden}"
        );
    }
}

#[test]
fn generated_bindings_stay_out_of_the_repository() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for leaked in ["generated-bindings", "vendor/disktree"] {
        assert!(
            !repo_root.join(leaked).exists(),
            "{leaked} must not be committed; bindings and vendored code stay out of the repo"
        );
    }
}
