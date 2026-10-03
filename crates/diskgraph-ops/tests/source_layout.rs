//! OP-14 源码门禁：检查所有平台的生产 AST，不以当前宿主的 cfg 编译代替组织检查。
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

fn unsupported_copy_cfg(meta: &syn::Meta) -> bool {
    let syn::Meta::List(not) = meta else {
        return false;
    };
    if !not.path.is_ident("not") {
        return false;
    }
    let Ok(syn::Meta::List(any)) = not.parse_args::<syn::Meta>() else {
        return false;
    };
    if !any.path.is_ident("any") {
        return false;
    }
    let Ok(children) = any.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    ) else {
        return false;
    };
    let values: BTreeSet<_> = children
        .iter()
        .filter_map(|meta| {
            let syn::Meta::NameValue(value) = meta else {
                return None;
            };
            if !value.path.is_ident("target_os") {
                return None;
            }
            let syn::Expr::Lit(lit) = &value.value else {
                return None;
            };
            let syn::Lit::Str(value) = &lit.lit else {
                return None;
            };
            Some(value.value())
        })
        .collect();
    children.len() == 2 && values == BTreeSet::from(["macos".into(), "linux".into()])
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
    objects: BTreeSet<String>,
    unsupported_copy_cleanup: bool,
    resource_free_copy: bool,
    violations: Vec<String>,
}

impl ProductionVisitor {
    fn type_doc(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        self.objects.insert(name.to_string());
        let doc = documentation(attributes);
        if !doc.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            || !doc.contains("来源：")
            || doc.contains("对应 Java:")
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

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let previous = self.unsupported_copy_cleanup;
        self.unsupported_copy_cleanup = self.resource_free_copy
            && item.trait_.is_none()
            && item.generics.params.is_empty()
            && item.generics.where_clause.is_none()
            && matches!(item.self_ty.as_ref(), syn::Type::Path(path) if path.path.is_ident("CrossVolumeCopy"))
            && item.attrs.iter().any(|attr| {
                attr.path().is_ident("cfg")
                    && attr
                        .parse_args::<syn::Meta>()
                        .is_ok_and(|meta| unsupported_copy_cfg(&meta))
            });
        syn::visit::visit_item_impl(self, item);
        self.unsupported_copy_cleanup = previous;
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if item.ident == "CrossVolumeCopy"
            && matches!(item.fields, syn::Fields::Unit)
            && item.attrs.iter().any(|attr| {
                attr.path().is_ident("cfg")
                    && attr
                        .parse_args::<syn::Meta>()
                        .is_ok_and(|meta| unsupported_copy_cfg(&meta))
            })
        {
            self.resource_free_copy = true;
        }
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_union(self, item);
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
        if test_only(&item.attrs) {
            return;
        }
        if !matches!(item.vis, syn::Visibility::Inherited) {
            self.callable_doc(&item.sig.ident, &item.attrs);
        }
        self.body(&item.sig.ident, &item.block);
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if test_only(&item.attrs) {
            return;
        }
        if !matches!(item.vis, syn::Visibility::Inherited) {
            self.callable_doc(&item.sig.ident, &item.attrs);
        }
        // 仅既存非支持平台零资源 inherent discard(&self) -> () 可以为空。
        let sig = &item.sig;
        let legacy_empty_cleanup = self.unsupported_copy_cleanup
            && sig.ident == "discard"
            && sig.inputs.len() == 1
            && matches!(sig.inputs.first(), Some(syn::FnArg::Receiver(receiver)) if receiver.reference.is_some() && receiver.mutability.is_none() && receiver.colon_token.is_none())
            && matches!(sig.output, syn::ReturnType::Default)
            && sig.generics.params.is_empty()
            && sig.generics.where_clause.is_none()
            && sig.asyncness.is_none()
            && sig.constness.is_none()
            && sig.unsafety.is_none()
            && sig.abi.is_none()
            && sig.variadic.is_none();
        if !legacy_empty_cleanup {
            self.body(&item.sig.ident, &item.block);
        }
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
        "lib" | "mod" => path.parent().unwrap().to_path_buf(),
        stem => path.parent().unwrap().join(stem),
    }
}

fn module_sources(
    items: &[syn::Item],
    base: &Path,
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
            module_sources(items, &base.join(name), production, mounted, sources);
        } else {
            let direct = base.join(format!("{name}.rs"));
            let path = if direct.exists() {
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
    let text = std::fs::read_to_string(path).unwrap();
    let ast = syn::parse_file(&text).unwrap();
    module_sources(&ast.items, &module_base(path), production, mounted, sources);
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

fn entry_violations(ast: &syn::File) -> Vec<String> {
    let mut violations = Vec::new();
    for item in &ast.items {
        if !matches!(item, syn::Item::Mod(_) | syn::Item::Use(_)) {
            violations.push("入口只允许模块声明/明确导出".into());
        }
        if let syn::Item::Use(item) = item
            && matches!(item.vis, syn::Visibility::Inherited)
        {
            violations.push("入口不得保留普通 use".into());
        }
        if let syn::Item::Mod(item) = item
            && item.content.is_some()
            && !test_only(&item.attrs)
        {
            violations.push("入口不得内联生产模块".into());
        }
    }
    violations
}

fn source_violations(ast: &syn::File) -> Vec<String> {
    let mut visitor = ProductionVisitor::default();
    visitor.visit_file(ast);
    if visitor.objects.len() > 1 {
        visitor
            .violations
            .push(format!("{}个对象共用文件", visitor.objects.len()));
    }
    visitor.violations
}

#[test]
fn all_platform_ops_sources_follow_the_rust_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut mounted = BTreeSet::new();
    let mut sources = Vec::new();
    source(&root.join("lib.rs"), true, &mut mounted, &mut sources);
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
            violations.extend(
                entry_violations(&ast)
                    .into_iter()
                    .map(|v| format!("{label}: {v}")),
            );
        }
        violations.extend(
            source_violations(&ast)
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
    assert_eq!(visitor.objects.len(), 3);
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
    assert_eq!(visitor.objects.len(), 2);
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
fn gate_only_exempts_the_existing_resource_free_unsupported_cleanup() {
    let ast = syn::parse_file(r#"
/// 零资源平台状态。来源：DiskGraph 原生 Rust。
#[cfg(not(any(target_os = "macos", target_os = "linux")))] struct CrossVolumeCopy;
#[cfg(not(any(target_os = "macos", target_os = "linux")))] impl CrossVolumeCopy { fn discard(&self) {} fn accidental(&self) {} }
#[cfg(windows)] impl CrossVolumeCopy { fn discard(&self) {} }
#[cfg(not(any(target_os = "macos", target_os = "linux")))] impl DifferentOwner { fn discard(&self) {} }
#[cfg(not(any(target_os = "macos", target_os = "linux")))] impl Cleaner for CrossVolumeCopy { fn discard(&self) {} }
#[cfg(not(any(target_os = "macos", target_os = "linux")))] impl CrossVolumeCopy { fn discard() {} fn discard(&self, _new: u64) {} }
"#).unwrap();
    let mut visitor = ProductionVisitor::default();
    visitor.visit_file(&ast);
    assert_eq!(visitor.violations.len(), 6);
    assert!(
        visitor
            .violations
            .iter()
            .any(|v| v.starts_with("accidental:"))
    );
    let resource_owner = syn::parse_file(r#"#[cfg(not(any(target_os="macos",target_os="linux")))] struct CrossVolumeCopy { resource: u64 } #[cfg(not(any(target_os="macos",target_os="linux")))] impl CrossVolumeCopy { fn discard(&self) {} }"#).unwrap();
    let mut visitor = ProductionVisitor::default();
    visitor.visit_file(&resource_owner);
    assert!(visitor.violations.iter().any(|v| v.starts_with("discard:")));
}

#[test]
fn gate_rejects_missing_chinese_contracts_fake_sources_and_entry_bodies() {
    let ast = syn::parse_file(
        r#"/// 对应 Java: NotReal
pub struct A;
pub fn run() { let _value = 1; }"#,
    )
    .unwrap();
    let mut visitor = ProductionVisitor::default();
    visitor.visit_file(&ast);
    assert_eq!(visitor.violations.len(), 2);
    assert_eq!(entry_violations(&ast).len(), 2);
    let invalid =
        syn::parse_file("use std::path::Path; mod hidden { fn body() { let _x = 1; } }").unwrap();
    assert_eq!(entry_violations(&invalid).len(), 2);
    assert!(
        entry_violations(
            &syn::parse_file("mod implementation; pub use implementation::Object;").unwrap()
        )
        .is_empty()
    );
}

#[test]
fn gate_rejects_a_union_grouped_with_another_object() {
    for union_doc in ["", "/// 原生联合表示。来源：DiskGraph 原生 Rust。\n"] {
        let ast = syn::parse_file(&format!("/// 原生结构。来源：DiskGraph 原生 Rust。\nstruct StructA;\n{union_doc}union UnionB {{ raw: u64 }}")).unwrap();
        assert!(
            source_violations(&ast)
                .iter()
                .any(|error| error.contains("2个对象共用文件")),
            "union 不能隐藏在另一对象文件中"
        );
    }
}

#[test]
fn gate_requires_chinese_native_source_for_an_isolated_union() {
    let ast = syn::parse_file("union UndocumentedUnion { raw: u64 }").unwrap();
    assert!(
        source_violations(&ast)
            .iter()
            .any(|error| error.starts_with("UndocumentedUnion: 缺少中文对象用途与实际来源")),
        "独立 union 也必须有真实中文来源"
    );
}
