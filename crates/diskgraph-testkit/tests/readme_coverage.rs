//! Every published crate must ship a README (P7 regression): crates.io
//! renders a crate's README as its landing page, and a missing one shows
//! "appears to have no README.md file" — the first thing a consumer sees.
//!
//! This test also checks that the file is referenced from the manifest and
//! that `cargo package` actually includes it, because a README that exists
//! in the working tree but is excluded from the published tarball is the
//! same absence with extra steps.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The crates published to crates.io. The vendored scanner is published
/// under its own name and has its provenance README.
const PUBLISHED: &[&str] = &[
    "diskgraph-disktree-core",
    "diskgraph-core",
    "diskgraph-store",
    "diskgraph-disktree",
    "diskgraph-testkit",
    "diskgraph-engine",
    "diskgraph-ops",
    "diskgraph-mcp",
    "diskgraph-cli",
    "diskgraph-ffi",
];

fn readme_of(crate_dir: &Path) -> String {
    let path = crate_dir.join("README.md");
    assert!(
        path.is_file(),
        "{} has no README.md; crates.io shows an empty landing page without one",
        crate_dir.display()
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("{} is unreadable: {error}", path.display());
    })
}

#[test]
fn every_published_crate_has_a_substantive_readme() {
    let root = workspace_root();
    for name in PUBLISHED {
        let dir = root.join("crates").join(name);
        assert!(
            dir.is_dir(),
            "{name} is listed as published but has no crate directory"
        );
        let readme = readme_of(&dir);
        assert!(
            readme.trim().lines().count() >= 12,
            "{name}/README.md is too thin to document the crate ({} lines)",
            readme.trim().lines().count()
        );
        // A README must say what it is, how to use it, and under what terms.
        let lowered = readme.to_lowercase();
        assert!(
            lowered.contains("install")
                || lowered.contains("add") && lowered.contains("dependencies"),
            "{name}/README.md must show how to depend on the crate"
        );
        assert!(
            lowered.contains("mit"),
            "{name}/README.md must state the license"
        );
    }
}

#[test]
fn the_manifest_references_the_readme_where_it_is_not_automatic() {
    // Cargo picks up README.md automatically for a crate in the same
    // directory; the assertion documents that expectation, and catches a
    // rename that would silently drop the landing page.
    for name in PUBLISHED {
        let dir = workspace_root().join("crates").join(name);
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        let declared = manifest.contains("readme =");
        let readme_exists = dir.join("README.md").is_file();
        assert!(
            readme_exists || declared,
            "{name} has neither a README.md nor a readme field"
        );
    }
}
