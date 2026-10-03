//! RT-09 FFI 源码门禁：检查所有平台的生产 AST，不以当前宿主的 cfg 编译代替组织检查。
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
            let meta = if attribute.path().is_ident("cfg_attr") {
                let syn::Meta::List(list) = &attribute.meta else {
                    return None;
                };
                let nested = list
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                    )
                    .ok()?;
                if nested.len() != 2
                    || !matches!(&nested[0], syn::Meta::Path(path) if path.is_ident("doc"))
                {
                    return None;
                }
                nested[1].clone()
            } else {
                attribute.meta.clone()
            };
            if let syn::Meta::NameValue(value) = meta
                && value.path.is_ident("doc")
                && let syn::Expr::Lit(value) = value.value
                && let syn::Lit::Str(value) = value.lit
            {
                Some(value.value())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn binding_include(mac: &syn::Macro) -> Option<String> {
    if !mac.path.is_ident("include") {
        return None;
    }
    let value = syn::parse2::<syn::LitStr>(mac.tokens.clone()).ok()?.value();
    matches!(value.as_str(), "api_exports.rs" | "job_handle.rs").then_some(value)
}

fn entry_violations(ast: &syn::File, library_root: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let mut scaffolds = 0;
    let mut includes = BTreeSet::new();
    for item in &ast.items {
        match item {
            syn::Item::Mod(item) => {
                if item.content.is_some() && !test_only(&item.attrs) {
                    errors.push("入口不得内联生产模块".into());
                }
            }
            syn::Item::Use(item) => {
                if matches!(item.vis, syn::Visibility::Inherited) {
                    errors.push("入口不得保留普通 use".into());
                }
            }
            syn::Item::Macro(value) if library_root => {
                let mac = &value.mac;
                if mac.path.segments.len() == 2
                    && mac.path.segments[0].ident == "uniffi"
                    && mac.path.segments[1].ident == "setup_scaffolding"
                    && mac.tokens.is_empty()
                {
                    scaffolds += 1;
                } else if let Some(name) = binding_include(mac) {
                    if !includes.insert(name) {
                        errors.push("重复 ABI 包含文件".into());
                    }
                } else {
                    errors.push("禁止非指定 ABI 文件或其他入口宏".into());
                }
            }
            _ => errors.push("入口只允许模块声明/明确导出及指定 ABI 宏".into()),
        }
    }
    if library_root
        && (scaffolds != 1
            || includes != BTreeSet::from(["api_exports.rs".into(), "job_handle.rs".into()]))
    {
        errors.push("必须恰有一个脚手架及两份真实 ABI 包含文件".into());
    }
    errors
}

#[derive(Default)]
struct ProductionVisitor {
    objects: usize,
    abi_entry: bool,
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
    fn visit_file(&mut self, file: &'ast syn::File) {
        // 仅实际根文件顶层的两份已挂载实现可以包含；函数/表达式/内联模块不能继承例外。
        for item in &file.items {
            if self.abi_entry
                && let syn::Item::Macro(value) = item
                && binding_include(&value.mac).is_some()
            {
                continue;
            }
            self.visit_item(item);
        }
    }

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
            .is_some_and(|segment| segment.ident == "include")
        {
            self.violations
                .push("禁止 include：仅根文件顶层固定 ABI 文件获准".into());
        }
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
    attribute_base: &Path,
    production: bool,
    abi_entry: bool,
    mounted: &mut BTreeSet<PathBuf>,
    sources: &mut Vec<(PathBuf, String, syn::File)>,
) {
    for item in items {
        if let syn::Item::Macro(value) = item
            && value.mac.path.is_ident("include")
        {
            assert!(abi_entry, "禁止 include：非根文件顶层");
            let name = binding_include(&value.mac).expect("只允许指定真实 ABI 包含文件");
            source(&attribute_base.join(name), production, mounted, sources);
            continue;
        }
        let syn::Item::Mod(module) = item else {
            continue;
        };
        let production = production && !test_only(&module.attrs);
        let name = module.ident.to_string();
        if let Some((_, items)) = &module.content {
            module_sources(
                items,
                &base.join(&name),
                &attribute_base.join(&name),
                production,
                false,
                mounted,
                sources,
            );
        } else {
            let explicit = module.attrs.iter().find_map(|attr| {
                if attr.path().is_ident("path")
                    && let syn::Meta::NameValue(value) = &attr.meta
                    && let syn::Expr::Lit(value) = &value.value
                    && let syn::Lit::Str(value) = &value.lit
                {
                    Some(attribute_base.join(value.value()))
                } else {
                    None
                }
            });
            let direct = base.join(format!("{name}.rs"));
            let path = if let Some(path) = explicit {
                path
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
    let text = std::fs::read_to_string(path).unwrap();
    let ast = syn::parse_file(&text).unwrap();
    module_sources(
        &ast.items,
        &module_base(path),
        path.parent().unwrap(),
        production,
        mounted.len() == 1 && path.file_name().is_some_and(|name| name == "lib.rs"),
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
fn all_platform_ffi_sources_follow_the_rust_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut mounted = BTreeSet::new();
    let mut sources = Vec::new();
    source(&root.join("lib.rs"), true, &mut mounted, &mut sources);
    let mut disk = BTreeSet::new();
    rust_sources(&root, &mut disk);
    for path in &disk {
        if path.parent() == Some(root.join("bin").as_path()) {
            source(path, true, &mut mounted, &mut sources);
        }
    }
    assert_eq!(disk, mounted, "存在未挂载或缺失的 Rust 源文件");
    let mut violations = Vec::new();
    for path in &mounted {
        for component in path.strip_prefix(&root).unwrap().components() {
            let component = component.as_os_str().to_str().unwrap();
            let name = component.strip_suffix(".rs").unwrap_or(component);
            if !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                violations.push(format!(
                    "{}: 源文件和目录必须使用 snake_case",
                    path.display()
                ));
            }
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
            let library_root = path == root.join("lib.rs");
            violations.extend(
                entry_violations(&ast, library_root)
                    .into_iter()
                    .map(|error| format!("{label}: {error}")),
            );
        }
        let mut visitor = ProductionVisitor {
            abi_entry: path == root.join("lib.rs"),
            ..ProductionVisitor::default()
        };
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

#[path = "source_layout/negative_cases.rs"]
mod negative_cases;
