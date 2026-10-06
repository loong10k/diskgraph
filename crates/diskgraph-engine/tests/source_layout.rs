//! RT-09 源码门禁：检查所有平台的生产 AST，不以当前宿主的 cfg 编译代替组织检查。
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::visit::Visit;

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Meta>()
                .is_ok_and(|meta| requires_test(&meta))
    })
}

fn requires_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
            let Ok(children) = list.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return false;
            };
            // all(test, …) 必须启用 test；any(test, windows) 不能隐藏原生生产对象。
            if list.path.is_ident("all") {
                children.iter().any(requires_test)
            } else {
                !children.is_empty() && children.iter().all(requires_test)
            }
        }
        // 对其他复杂表达式保守检查，不能因当前宿主 cfg 为假而跳过生产源码。
        _ => false,
    }
}

fn documentation(attributes: &[syn::Attribute]) -> String {
    attributes
        .iter()
        .filter_map(|attribute| {
            if attribute.path().is_ident("doc")
                && let syn::Meta::NameValue(value) = &attribute.meta
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

#[derive(Default)]
struct ProductionVisitor {
    objects: usize,
    violations: Vec<String>,
}

impl ProductionVisitor {
    fn type_doc(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        self.objects += 1;
        let doc = documentation(attributes);
        if !doc.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            || !(doc.contains("来源：") || doc.contains("对应 Java:"))
        {
            self.violations
                .push(format!("{name}: 缺少中文对象用途与实际来源"));
        }
    }

    fn callable_doc(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        let doc = documentation(attributes);
        if !doc.contains("参数：") || !doc.contains("返回：") {
            self.violations
                .push(format!("{name}: 缺少中文参数/返回契约"));
        }
        if doc.contains("签名所示") || doc.contains("UNRESOLVED") {
            self.violations.push(format!("{name}: 禁止占位文档"));
        }
    }

    fn body(&mut self, name: &syn::Ident, body: &syn::Block) {
        if body.stmts.is_empty() {
            self.violations.push(format!("{name}: 禁止空函数实现"));
        }
    }
}

impl<'ast> Visit<'ast> for ProductionVisitor {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_trait(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !matches!(item.vis, syn::Visibility::Inherited) {
            self.callable_doc(&item.sig.ident, &item.attrs);
        }
        self.body(&item.sig.ident, &item.block);
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !matches!(item.vis, syn::Visibility::Inherited) {
            self.callable_doc(&item.sig.ident, &item.attrs);
        }
        self.body(&item.sig.ident, &item.block);
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.callable_doc(&item.sig.ident, &item.attrs);
        if let Some(body) = &item.default {
            self.body(&item.sig.ident, body);
        }
        syn::visit::visit_trait_item_fn(self, item);
    }

    fn visit_use_glob(&mut self, _glob: &'ast syn::UseGlob) {
        self.violations.push("生产 wildcard import".into());
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        if item
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "todo" || segment.ident == "unimplemented")
        {
            self.violations.push("生产占位宏".into());
        }
        syn::visit::visit_macro(self, item);
    }
}

fn module_base(path: &Path) -> PathBuf {
    match path.file_stem().unwrap().to_str().unwrap() {
        "lib" | "main" | "mod" => path.parent().unwrap().to_path_buf(),
        stem => path.parent().unwrap().join(stem),
    }
}

fn module_sources(
    items: &[syn::Item],
    base: &Path,
    declaration_directory: &Path,
    production: bool,
    mounted: &mut BTreeSet<PathBuf>,
    sources: &mut Vec<(PathBuf, String, syn::File)>,
) {
    for item in items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        let production = production && !test_only(&module.attrs);
        let name = module.ident.to_string();
        if let Some((_, items)) = &module.content {
            module_sources(
                items,
                &base.join(&name),
                &base.join(name),
                production,
                mounted,
                sources,
            );
        } else {
            let direct = base.join(format!("{name}.rs"));
            let explicit = module.attrs.iter().find_map(|attribute| {
                if attribute.path().is_ident("path")
                    && let syn::Meta::NameValue(value) = &attribute.meta
                    && let syn::Expr::Lit(value) = &value.value
                    && let syn::Lit::Str(value) = &value.lit
                {
                    Some(PathBuf::from(value.value()))
                } else {
                    None
                }
            });
            let path = if let Some(explicit) = explicit {
                assert!(
                    !explicit.is_absolute()
                        && explicit
                            .components()
                            .all(|component| matches!(component, std::path::Component::Normal(_))),
                    "模块path必须保持在声明目录内"
                );
                declaration_directory.join(explicit)
            } else if direct.exists() {
                direct
            } else {
                base.join(name).join("mod.rs")
            };
            source(&path, production, mounted, sources);
        }
    }
}

fn source(
    path: &Path,
    production: bool,
    mounted: &mut BTreeSet<PathBuf>,
    sources: &mut Vec<(PathBuf, String, syn::File)>,
) {
    assert!(
        mounted.insert(path.to_path_buf()),
        "重复挂载 {}",
        path.display()
    );
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("无法读取挂载源码 {}: {error}", path.display()));
    let ast = syn::parse_file(&text).unwrap();
    module_sources(
        &ast.items,
        &module_base(path),
        path.parent().unwrap(),
        production,
        mounted,
        sources,
    );
    if production {
        sources.push((path.to_path_buf(), text, ast));
    }
}

fn rust_sources(path: &Path, out: &mut BTreeSet<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.insert(path);
        }
    }
}

#[test]
fn all_platform_engine_sources_follow_the_rust_contract() {
    check_rust_contract(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
}

#[test]
fn scan_worker_sources_follow_the_same_rust_contract() {
    check_rust_contract(Path::new(env!("CARGO_MANIFEST_DIR")).join("../diskgraph-scan-worker/src"));
}

fn check_rust_contract(root: PathBuf) {
    let mut mounted = BTreeSet::new();
    let mut sources = Vec::new();
    source(&root.join("lib.rs"), true, &mut mounted, &mut sources);
    if root.join("main.rs").exists() {
        source(&root.join("main.rs"), true, &mut mounted, &mut sources);
    }
    let mut disk = BTreeSet::new();
    rust_sources(&root, &mut disk);
    assert_eq!(disk, mounted, "存在未挂载或缺失的 Rust 源文件");
    let mut violations = Vec::new();
    for path in &mounted {
        for component in path.strip_prefix(&root).unwrap().components() {
            let component = component.as_os_str().to_str().unwrap();
            let name = component.strip_suffix(".rs").unwrap_or(component);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "源文件和目录必须使用 snake_case: {}",
                path.display()
            );
        }
    }
    for (path, text, ast) in sources {
        let label = path.strip_prefix(&root).unwrap().display().to_string();
        if text.lines().count() >= 500 {
            violations.push(format!("{label}: 生产文件必须少于500行"));
        }
        if matches!(
            path.file_stem().and_then(|name| name.to_str()),
            Some("lib" | "mod")
        ) {
            for item in &ast.items {
                if !matches!(item, syn::Item::Mod(_) | syn::Item::Use(_)) {
                    violations.push(format!("{label}: 入口只允许模块声明/明确导出"));
                }
                if let syn::Item::Use(item) = item
                    && matches!(item.vis, syn::Visibility::Inherited)
                {
                    violations.push(format!("{label}: 入口不得保留普通 use"));
                }
                if let syn::Item::Mod(item) = item
                    && item.content.is_some()
                    && !test_only(&item.attrs)
                {
                    violations.push(format!("{label}: 入口不得内联生产模块"));
                }
            }
        }
        let mut visitor = ProductionVisitor::default();
        visitor.visit_file(&ast);
        if visitor.objects > 1 {
            violations.push(format!("{label}: {}个对象共用文件", visitor.objects));
        }
        violations.extend(
            visitor
                .violations
                .into_iter()
                .map(|error| format!("{label}: {error}")),
        );
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn gate_rejects_hidden_objects_private_stubs_and_undocumented_trait_methods() {
    let ast = syn::parse_file("mod hidden { struct A; enum B {} } fn stub() {} trait Probe { fn inspect(&self); } fn later() { std::todo!() }").unwrap();
    let mut visitor = ProductionVisitor::default();
    visitor.visit_file(&ast);
    assert_eq!(visitor.objects, 3);
    assert!(
        visitor
            .violations
            .iter()
            .any(|error| error.contains("stub: 禁止空"))
    );
    assert!(
        visitor
            .violations
            .iter()
            .any(|error| error.contains("inspect: 缺少"))
    );
    assert!(
        visitor
            .violations
            .iter()
            .any(|error| error.contains("占位宏"))
    );
}

#[test]
fn gate_excludes_only_explicit_test_modules() {
    let ast = syn::parse_file("#[cfg(test)] mod tests { use super::*; struct A; fn fixture() {} } #[cfg(all(test, not(unix)))] mod tests2 { use super::*; struct D; fn fixture() {} } #[cfg(windows)] mod native { struct B; } #[cfg(any(test, windows))] mod mixed { struct C; }").unwrap();
    let mut visitor = ProductionVisitor::default();
    visitor.visit_file(&ast);
    assert_eq!(visitor.objects, 2);
    assert_eq!(visitor.violations.len(), 2);
    assert!(
        visitor
            .violations
            .iter()
            .any(|error| error.starts_with("B:"))
    );
    assert!(
        visitor
            .violations
            .iter()
            .any(|error| error.starts_with("C:"))
    );
}

#[test]
fn gate_mounts_path_attributes_relative_to_the_declaring_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("lib.rs"), "mod owner;").unwrap();
    std::fs::write(
        root.join("owner.rs"),
        "#[path = \"sibling.rs\"] mod hidden; mod regular;",
    )
    .unwrap();
    std::fs::write(root.join("sibling.rs"), "fn actual_logic() -> u8 { 1 }").unwrap();
    std::fs::create_dir(root.join("owner")).unwrap();
    std::fs::write(
        root.join("owner/regular.rs"),
        "fn other_logic() -> u8 { 2 }",
    )
    .unwrap();
    let mut mounted = BTreeSet::new();
    let mut sources = Vec::new();
    source(&root.join("lib.rs"), true, &mut mounted, &mut sources);
    assert_eq!(mounted.len(), 4);
    assert!(mounted.contains(&root.join("sibling.rs")));
    assert!(mounted.contains(&root.join("owner/regular.rs")));
    assert_eq!(sources.len(), 4);
}

#[test]
fn gate_rejects_explicit_module_path_escape_before_reading() {
    let temp = tempfile::tempdir().unwrap();
    for path in ["../outside.rs", "/outside.rs"] {
        std::fs::write(
            temp.path().join("lib.rs"),
            format!("#[path = {path:?}] mod outside;"),
        )
        .unwrap();
        let failure = std::panic::catch_unwind(|| {
            source(
                &temp.path().join("lib.rs"),
                true,
                &mut BTreeSet::new(),
                &mut Vec::new(),
            );
        })
        .expect_err("escaping source must be rejected");
        let message = failure
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| failure.downcast_ref::<&str>().copied())
            .unwrap();
        assert!(
            message.contains("模块path必须保持在声明目录内"),
            "wrong refusal: {message}"
        );
    }
}
