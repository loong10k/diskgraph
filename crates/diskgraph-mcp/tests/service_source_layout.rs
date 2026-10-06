//! MCP 实际挂载源码的增量结构门禁；来源：OpenSpec RT-10 / D40 原生 Rust 契约。
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

// 本批不重构这些既有适配/传输模块；明确豁免不等于已经满足整 crate 结构规范。
// D42 的 evidence_job_status 已统一 Git/进程投影，按新模块执行完整结构检查，不列为旧模块豁免。
const LEGACY_MODULES: &[&str] = &[
    "auth",
    "bounded_json_writer",
    "children_cursor_tests",
    "client_address",
    "connection_rejection",
    "doctor",
    "error_reply",
    "history_budget_tests",
    "http",
    "install",
    "job_entry",
    "legacy",
    "legacy_delivery_error",
    "legacy_delivery_registry",
    "legacy_delivery_session",
    "legacy_delivery_state",
    "legacy_frame",
    "legacy_reservation",
    "legacy_session_receiver",
    "legacy_transport",
    "protocol",
    "rate_limit_state",
    "rate_limiter",
    "relation_budget_tests",
    "relation_reply",
    "request_authorizer",
    "request_context",
    "snapshot_reply",
    "sse_slot",
    "token_bucket",
    "tool_input_schema",
];
const PUBLIC_MODULES: &[&str] = &["auth", "doctor", "http", "install", "legacy", "protocol"];

// 原入口以外的既有文件：前三项由原适配模块挂载，main 是独立二进制入口。
const LEGACY_EXTRA_FILES: &[&str] = &[
    "auth_key_acl.rs",
    "legacy_budget_tests.rs",
    "legacy_transport_tests.rs",
    "main.rs",
];
fn aggregate_item(item: &syn::Item) -> bool {
    match item {
        syn::Item::Mod(_) => true,
        syn::Item::Use(value) => !matches!(value.vis, syn::Visibility::Inherited),
        _ => false,
    }
}
fn orphan_sources(
    root: &Path,
    mounted: &BTreeSet<PathBuf>,
    legacy: &BTreeSet<PathBuf>,
) -> Vec<PathBuf> {
    let mut pending = vec![root.to_owned()];
    let mut orphans = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                orphans.push(path);
            } else if kind.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && !mounted.contains(&path)
                && !legacy.contains(&path)
            {
                orphans.push(path);
            }
        }
    }
    orphans.sort();
    orphans
}
fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

// 条件路径也会改变实际源；无法解析条件属性时拒绝猜测挂载。
fn hidden_path(meta: &syn::Meta) -> bool {
    if meta.path().is_ident("path") {
        return true;
    }
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let syn::Meta::List(list) = meta else {
        return true;
    };
    list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        .map_or(true, |parts| {
            parts.len() < 2 || parts.iter().skip(1).any(hidden_path)
        })
}
fn docs(attributes: &[syn::Attribute]) -> String {
    attributes
        .iter()
        .filter_map(|attribute| {
            if let syn::Meta::NameValue(value) = &attribute.meta
                && value.path.is_ident("doc")
                && let syn::Expr::Lit(value) = &value.value
                && let syn::Lit::Str(value) = &value.lit
            {
                Some(value.value())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn chinese(text: &str) -> bool {
    text.chars()
        .any(|value| ('\u{4e00}'..='\u{9fff}').contains(&value))
}

/// 逐实际 AST 检查职责、公开契约和占位实现；来源：MCP RT-10 结构验收。
#[derive(Default)]
struct SourceVisitor {
    test_source: bool,
    objects: Vec<String>,
    errors: Vec<String>,
}

impl SourceVisitor {
    fn object(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        self.objects.push(name.to_string());
        let text = docs(attributes);
        if !chinese(&text) || !text.contains("来源：") {
            self.errors
                .push(format!("{name}: 对象缺少中文用途及原生来源"));
        }
    }

    fn function(
        &mut self,
        name: &syn::Ident,
        visibility: &syn::Visibility,
        attributes: &[syn::Attribute],
        block: &syn::Block,
    ) {
        if block.stmts.is_empty() {
            self.errors.push(format!("{name}: 空函数占位"));
        }
        if !self.test_source && !matches!(visibility, syn::Visibility::Inherited) {
            let text = docs(attributes);
            if !chinese(&text) || !text.contains("参数") || !text.contains("返回") {
                self.errors
                    .push(format!("{name}: 公开方法缺少中文参数/返回契约"));
            }
        }
    }
}

impl<'ast> Visit<'ast> for SourceVisitor {
    fn visit_item_mod(&mut self, value: &'ast syn::ItemMod) {
        if value.content.is_some() {
            self.errors.push("内联模块隐藏实现".into());
        }
        if value
            .attrs
            .iter()
            .any(|attribute| hidden_path(&attribute.meta))
        {
            self.errors.push("路径覆盖隐藏挂载".into());
        }
        if !test_only(&value.attrs) {
            syn::visit::visit_item_mod(self, value);
        }
    }
    fn visit_item_struct(&mut self, value: &'ast syn::ItemStruct) {
        self.object(&value.ident, &value.attrs);
        syn::visit::visit_item_struct(self, value);
    }
    fn visit_item_enum(&mut self, value: &'ast syn::ItemEnum) {
        self.object(&value.ident, &value.attrs);
        syn::visit::visit_item_enum(self, value);
    }
    fn visit_item_type(&mut self, value: &'ast syn::ItemType) {
        self.object(&value.ident, &value.attrs);
        syn::visit::visit_item_type(self, value);
    }
    fn visit_item_trait(&mut self, value: &'ast syn::ItemTrait) {
        self.object(&value.ident, &value.attrs);
        syn::visit::visit_item_trait(self, value);
    }
    fn visit_item_union(&mut self, value: &'ast syn::ItemUnion) {
        self.object(&value.ident, &value.attrs);
        syn::visit::visit_item_union(self, value);
    }
    fn visit_item_fn(&mut self, value: &'ast syn::ItemFn) {
        self.function(&value.sig.ident, &value.vis, &value.attrs, &value.block);
        syn::visit::visit_item_fn(self, value);
    }
    fn visit_impl_item_fn(&mut self, value: &'ast syn::ImplItemFn) {
        self.function(&value.sig.ident, &value.vis, &value.attrs, &value.block);
        syn::visit::visit_impl_item_fn(self, value);
    }
    fn visit_use_glob(&mut self, _: &'ast syn::UseGlob) {
        if !self.test_source {
            self.errors.push("生产 wildcard import".into());
        }
    }
    fn visit_macro(&mut self, value: &'ast syn::Macro) {
        if value.path.segments.last().is_some_and(|segment| {
            matches!(
                segment.ident.to_string().as_str(),
                "todo" | "unimplemented" | "include"
            )
        }) {
            self.errors.push("占位或 include 隐藏实现".into());
        }
        syn::visit::visit_macro(self, value);
    }
}

fn inspect(
    path: &Path,
    test_source: bool,
    mounted: &mut BTreeSet<PathBuf>,
    errors: &mut Vec<String>,
) {
    assert!(
        mounted.insert(path.to_owned()),
        "重复挂载 {}",
        path.display()
    );
    let source = std::fs::read_to_string(path).unwrap();
    let ast = syn::parse_file(&source).unwrap();
    let entry = path.file_name().is_some_and(|name| name == "lib.rs");
    let aggregate = entry || path.file_name().is_some_and(|name| name == "mod.rs");
    if source.lines().count() >= 500 {
        errors.push(format!("{}: 实际源必须少于500行", path.display()));
    }
    let mut visitor = SourceVisitor {
        test_source,
        ..SourceVisitor::default()
    };
    visitor.visit_file(&ast);
    if visitor.objects.len() > usize::from(!aggregate) {
        errors.push(format!(
            "{}: 聚合含对象或独立源多对象 {:?}",
            path.display(),
            visitor.objects
        ));
    }
    if aggregate && ast.items.iter().any(|item| !aggregate_item(item)) {
        errors.push(format!(
            "{}: 聚合入口只能声明模块和显式重导出",
            path.display()
        ));
    }
    if entry {
        let public: BTreeSet<_> = ast
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Mod(module) if matches!(module.vis, syn::Visibility::Public(_)) => {
                    Some(module.ident.to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            public,
            PUBLIC_MODULES.iter().map(|name| name.to_string()).collect(),
            "原公开模块路径必须保持"
        );
        for name in LEGACY_MODULES {
            assert!(
                ast.items.iter().any(|item| matches!(item,
                    syn::Item::Mod(module) if module.ident == *name && module.content.is_none()
                )),
                "既有模块{name}不得移除或用内联替代"
            );
        }
    }
    errors.extend(
        visitor
            .errors
            .into_iter()
            .map(|error| format!("{}: {error}", path.display())),
    );
    let base = if aggregate || path.file_stem().is_some_and(|stem| stem == "main") {
        path.parent().unwrap().to_owned()
    } else {
        path.with_extension("")
    };
    for item in ast.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if module.content.is_some() || module.attrs.iter().any(|attr| hidden_path(&attr.meta)) {
            continue;
        }
        let name = module.ident.to_string();
        if entry && LEGACY_MODULES.contains(&name.as_str()) {
            continue;
        }
        let direct = base.join(format!("{name}.rs"));
        let child = if direct.exists() {
            direct
        } else {
            base.join(name).join("mod.rs")
        };
        assert!(child.is_file(), "真实标准模块未挂载 {}", child.display());
        inspect(
            &child,
            test_source || test_only(&module.attrs),
            mounted,
            errors,
        );
    }
}

#[test]
fn service_entry_and_mounted_modules_have_real_responsibilities() {
    let mut mounted = BTreeSet::new();
    let mut errors = Vec::new();
    inspect(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        false,
        &mut mounted,
        &mut errors,
    );
    // 二进制退出模块也必须实际挂载检查，不能因只读lib入口就成为孤儿例外。
    inspect(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        false,
        &mut mounted,
        &mut errors,
    );
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let legacy: BTreeSet<_> = LEGACY_MODULES
        .iter()
        .map(|name| source_root.join(format!("{name}.rs")))
        .chain(LEGACY_EXTRA_FILES.iter().map(|name| source_root.join(name)))
        .collect();
    errors.extend(
        orphan_sources(&source_root, &mounted, &legacy)
            .into_iter()
            .map(|path| format!("{}: 新增源未实际挂载", path.display())),
    );
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn root_public_types_defaults_and_stdio_entry_remain_importable() {
    use diskgraph_mcp::{McpConfig, McpService, STDIO_PRINCIPAL, serve_stdio};
    let _: fn(McpConfig) -> Result<McpService, diskgraph_engine::EngineError> = McpService::open;
    let _: fn(McpConfig) -> Result<McpService, diskgraph_engine::EngineError> =
        McpService::open_remote;
    let _stdio = serve_stdio::<std::io::Cursor<Vec<u8>>, Vec<u8>, Vec<u8>>;
    let config = McpConfig::default();
    assert_eq!(STDIO_PRINCIPAL, "local-user");
    assert_eq!(config.principal.as_str(), STDIO_PRINCIPAL);
    assert_eq!(config.data_dir, PathBuf::from("diskgraph-data"));
    assert_eq!(
        config.profile,
        diskgraph_mcp::protocol::ToolProfile::ReadFull
    );
    assert!(!config.legacy_sse);
}

#[test]
fn gate_rejects_hidden_paths_multiple_objects_stubs_and_missing_contracts() {
    let source = r#"#[path="elsewhere.rs"] mod hidden;
        mod inline { struct A; type B=u8; fn empty(){} use crate::*;
            fn fake(){todo!()} fn other(){unimplemented!()} fn hidden(){include!("body.rs")} }
        pub fn undocumented() { let _value = 1; }"#;
    let mut visitor = SourceVisitor::default();
    visitor.visit_file(&syn::parse_file(source).unwrap());
    assert_eq!(visitor.objects, ["A", "B"]);
    for diagnostic in [
        "内联模块",
        "路径覆盖",
        "空函数",
        "wildcard",
        "隐藏实现",
        "中文用途",
        "参数/返回",
    ] {
        assert!(
            visitor
                .errors
                .iter()
                .any(|error| error.contains(diagnostic)),
            "未拒绝{diagnostic}"
        );
    }
}

#[test]
fn gate_accepts_real_documented_bodies_without_confusing_empty_match_arms() {
    let source = "/// 原生状态。来源：DiskGraph Rust。\nstruct State;\nimpl State {\n/// 执行检查。参数：无。返回：无。\npub fn check() { match true { true => {}, false => {} } } }";
    let mut visitor = SourceVisitor::default();
    visitor.visit_file(&syn::parse_file(source).unwrap());
    assert_eq!(visitor.objects, ["State"]);
    assert!(visitor.errors.is_empty(), "{:?}", visitor.errors);
}

#[test]
fn aggregate_entry_accepts_explicit_reexports_and_rejects_private_imports() {
    let public =
        syn::parse_file("mod mounted; pub use mounted::State; pub(crate) use mounted::helper;")
            .unwrap();
    assert!(public.items.iter().all(aggregate_item));
    let private = syn::parse_file("mod mounted; use mounted::State;").unwrap();
    assert!(!private.items.iter().all(aggregate_item));
}

#[test]
fn filesystem_gate_finds_root_and_nested_orphans_but_accepts_mounted_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(
        root.join("mod.rs"),
        "mod mounted; pub(crate) use mounted::VALUE;",
    )
    .unwrap();
    std::fs::write(root.join("mounted.rs"), "pub const VALUE:u8=1;").unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
    std::fs::write(root.join("orphan.rs"), "fn unused() {}").unwrap();
    std::fs::create_dir(root.join("hidden")).unwrap();
    std::fs::write(root.join("hidden/orphan.rs"), "fn unused() {}").unwrap();
    let mut mounted = BTreeSet::new();
    let mut errors = Vec::new();
    inspect(&root.join("mod.rs"), false, &mut mounted, &mut errors);
    assert!(errors.is_empty(), "{errors:?}");
    let legacy = BTreeSet::from([root.join("main.rs")]);
    assert_eq!(
        orphan_sources(root, &mounted, &legacy),
        vec![root.join("hidden/orphan.rs"), root.join("orphan.rs")]
    );
    std::fs::remove_file(root.join("orphan.rs")).unwrap();
    std::fs::remove_file(root.join("hidden/orphan.rs")).unwrap();
    assert!(orphan_sources(root, &mounted, &legacy).is_empty());
}

// 真实标准目录递归夹具：替代源存在，不能仅检查根 AST 后误读标准同名文件。
fn inspect_conditional_module(attribute: &str) -> (Vec<String>, bool) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("mod.rs"), "mod nested;").unwrap();
    std::fs::write(
        root.join("nested.rs"),
        format!("{attribute} mod disguised;"),
    )
    .unwrap();
    std::fs::create_dir(root.join("nested")).unwrap();
    std::fs::write(root.join("nested/disguised.rs"), "const VALUE:u8=1;").unwrap();
    std::fs::write(root.join("nested/legacy.rs"), "const VALUE:u8=2;").unwrap();
    let mut mounted = BTreeSet::new();
    let mut errors = Vec::new();
    inspect(&root.join("mod.rs"), false, &mut mounted, &mut errors);
    (errors, mounted.contains(&root.join("nested/disguised.rs")))
}

#[test]
fn filesystem_gate_rejects_direct_cfg_attr_path_override() {
    for attribute in [
        r#"#[cfg_attr(not(test), path="legacy.rs")]"#,
        "#[cfg_attr(not(test), path=)]",
    ] {
        let (errors, child_mounted) = inspect_conditional_module(attribute);
        assert!(
            errors.iter().any(|error| error.contains("路径覆盖")),
            "{errors:?}"
        );
        assert!(!child_mounted, "路径不可信时不得误读标准子模块");
    }
}

#[test]
fn filesystem_gate_rejects_nested_cfg_attr_path_override() {
    let (errors, child_mounted) =
        inspect_conditional_module(r#"#[cfg_attr(not(test), cfg_attr(unix, path="legacy.rs"))]"#);
    assert!(
        errors.iter().any(|error| error.contains("路径覆盖")),
        "{errors:?}"
    );
    assert!(!child_mounted, "嵌套路径覆盖不得误读标准子模块");
}

#[test]
fn filesystem_gate_accepts_safe_cfg_attr_and_visits_standard_child() {
    let (errors, child_mounted) =
        inspect_conditional_module("#[cfg_attr(not(test), cfg_attr(unix, allow(dead_code)))]");
    assert!(errors.is_empty(), "{errors:?}");
    assert!(child_mounted, "安全条件属性必须继续检查真实标准子模块");
}

#[test]
fn http_request_has_real_logic_in_own_module_and_preserves_public_path() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let source = std::fs::read_to_string(directory.join("http_request.rs"))
        .expect("HttpRequest must have a dedicated real implementation module");
    assert!(source.contains("pub struct HttpRequest"));
    assert!(source.contains("pub fn header("));
    assert!(source.contains("pub fn query_param("));
    let transport = std::fs::read_to_string(directory.join("http.rs")).unwrap();
    assert!(!transport.contains("pub struct HttpRequest"));
    assert!(transport.contains("pub use crate::http_request::HttpRequest;"));
    let request = diskgraph_mcp::http::HttpRequest {
        method: "GET".into(),
        path: "/mcp".into(),
        query: "session=%2Fraw&session=second&empty=".into(),
        headers: std::collections::HashMap::from([(
            "authorization".into(),
            "Bearer opaque".into(),
        )]),
        body: String::new(),
    };
    assert_eq!(request.header("AUTHORIZATION"), Some("Bearer opaque"));
    assert_eq!(request.query_param("session"), Some("%2Fraw".into()));
    assert_eq!(request.query_param("empty"), Some(String::new()));
    assert_eq!(request.query_param("missing"), None);
    assert_eq!(request, request.clone());
}
