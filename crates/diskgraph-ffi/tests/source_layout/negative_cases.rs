//! ABI 包含与 rustdoc 条件的负向门禁。
use super::{ProductionVisitor, entry_violations, source};
use std::collections::BTreeSet;
use syn::visit::Visit;

#[test]
fn gate_rejects_include_in_blocks_expressions_and_other_modules() {
    for text in [
        "fn probe() { include!(\"../../outside.rs\"); }",
        "fn probe() -> u8 { include!(\"api_exports.rs\") }",
        "mod nested { include!(\"job_handle.rs\"); }",
        "include!(\"api_exports.rs\");",
        "fn probe() { std::include!(\"../../outside.rs\"); }",
    ] {
        let mut visitor = ProductionVisitor::default();
        visitor.visit_file(&syn::parse_file(text).unwrap());
        assert!(
            visitor
                .violations
                .iter()
                .any(|error| error.contains("禁止 include")),
            "未拒绝非根顶层包含：{text}"
        );
    }
}

#[test]
fn gate_rejects_same_named_include_from_non_root_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("nested")).unwrap();
    std::fs::write(root.join("lib.rs"), "mod nested;").unwrap();
    std::fs::write(root.join("nested.rs"), "include!(\"api_exports.rs\");").unwrap();
    std::fs::write(root.join("api_exports.rs"), "fn hidden() { 1; }").unwrap();
    let rejected = std::panic::catch_unwind(|| {
        source(
            &root.join("lib.rs"),
            true,
            &mut BTreeSet::new(),
            &mut Vec::new(),
        );
    });
    assert!(rejected.is_err(), "非根文件不得挂载同名 ABI include");
}

#[test]
fn gate_permits_only_exact_abi_includes_and_one_scaffolding() {
    let valid =
        "include!(\"api_exports.rs\"); include!(\"job_handle.rs\"); uniffi::setup_scaffolding!();";
    assert!(entry_violations(&syn::parse_file(valid).unwrap(), true).is_empty());
    for invalid in [
        "",
        "include!(\"compat.rs\");",
        "include!(concat!(\"api_exports.rs\"));",
        "uniffi::other!();",
    ] {
        assert!(!entry_violations(&syn::parse_file(invalid).unwrap(), true).is_empty());
    }
    let duplicate = format!("{valid} uniffi::setup_scaffolding!();");
    assert!(!entry_violations(&syn::parse_file(&duplicate).unwrap(), true).is_empty());
    assert!(!entry_violations(&syn::parse_file(valid).unwrap(), false).is_empty());
}

#[test]
fn gate_reads_chinese_rustdoc_only_under_exact_doc_cfg() {
    for (condition, expected) in [("doc", 0), ("unix", 2)] {
        let source = format!(
            "#[cfg_attr({condition}, doc = \"中文用途 来源：实际 Rust\")] struct One; #[cfg_attr({condition}, doc = \"参数：无；返回：整数\")] pub fn run() -> u8 {{ 1 }}"
        );
        let mut visitor = ProductionVisitor::default();
        visitor.visit_file(&syn::parse_file(&source).unwrap());
        assert_eq!(visitor.objects, 1);
        assert_eq!(visitor.violations.len(), expected);
    }
}

#[test]
fn gate_mounts_explicit_path_and_include_as_real_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("lib.rs"), "#[path=\"actual.rs\"] mod aliased; include!(\"api_exports.rs\"); include!(\"job_handle.rs\"); uniffi::setup_scaffolding!();").unwrap();
    for (name, text) in [
        ("actual.rs", "struct One;"),
        ("api_exports.rs", "pub fn hidden_stub() {}"),
        ("job_handle.rs", "struct Handle;"),
    ] {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut mounted = BTreeSet::new();
    let mut sources = Vec::new();
    source(&root.join("lib.rs"), true, &mut mounted, &mut sources);
    assert_eq!(mounted.len(), 4);
    assert_eq!(sources.len(), 4);
    let mut visitor = ProductionVisitor::default();
    for (_, _, ast) in sources {
        visitor.visit_file(&ast);
    }
    assert_eq!(visitor.objects, 2);
    assert!(
        visitor
            .violations
            .iter()
            .any(|error| error.contains("hidden_stub: 禁止空"))
    );
    let duplicate = std::panic::catch_unwind(|| {
        let mut mounted = BTreeSet::new();
        let mut sources = Vec::new();
        source(
            &root.join("api_exports.rs"),
            true,
            &mut mounted,
            &mut sources,
        );
        source(
            &root.join("api_exports.rs"),
            true,
            &mut mounted,
            &mut sources,
        );
    });
    assert!(duplicate.is_err());
}
