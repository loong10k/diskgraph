//! D33 源码组织门禁：仅覆盖历史尺寸、query 与 compare 的实际生产实现。
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
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

#[derive(Default)]
struct SourceVisitor {
    objects: usize,
    errors: Vec<String>,
}

impl SourceVisitor {
    fn object(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        self.objects += 1;
        let doc = docs(attributes);
        if !doc.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            || !(doc.contains("来源：") || doc.contains("对应 Java:"))
        {
            self.errors.push(format!("{name}: 缺少中文用途及真实来源"));
        }
    }

    fn callable(
        &mut self,
        name: &syn::Ident,
        attributes: &[syn::Attribute],
        visibility: &syn::Visibility,
    ) {
        let doc = docs(attributes);
        if !matches!(visibility, syn::Visibility::Inherited)
            && (!doc.contains("参数：") || !doc.contains("返回："))
        {
            self.errors.push(format!("{name}: 缺少中文参数/返回说明"));
        }
    }
}

impl<'ast> Visit<'ast> for SourceVisitor {
    fn visit_item_mod(&mut self, value: &'ast syn::ItemMod) {
        if !test_only(&value.attrs) {
            if value
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("path"))
            {
                self.errors
                    .push("禁止生产模块路径覆盖；必须检查约定的真实源文件".into());
            }
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
    fn visit_item_fn(&mut self, value: &'ast syn::ItemFn) {
        self.callable(&value.sig.ident, &value.attrs, &value.vis);
        if value.block.stmts.is_empty() {
            self.errors.push("空实现".into());
        }
        syn::visit::visit_item_fn(self, value);
    }
    fn visit_impl_item_fn(&mut self, value: &'ast syn::ImplItemFn) {
        self.callable(&value.sig.ident, &value.attrs, &value.vis);
        if value.block.stmts.is_empty() {
            self.errors.push("空实现".into());
        }
        syn::visit::visit_impl_item_fn(self, value);
    }
    fn visit_use_glob(&mut self, _: &'ast syn::UseGlob) {
        self.errors.push("生产 wildcard import".into());
    }
    fn visit_macro(&mut self, value: &'ast syn::Macro) {
        if value.path.segments.last().is_some_and(|part| {
            matches!(
                part.ident.to_string().as_str(),
                "todo" | "unimplemented" | "include"
            )
        }) {
            self.errors.push("禁止占位或隐藏源的宏".into());
        }
        syn::visit::visit_macro(self, value);
    }
}

fn inspect(path: &Path, mounted: &mut BTreeSet<PathBuf>, errors: &mut Vec<String>) {
    assert!(
        mounted.insert(path.to_owned()),
        "重复挂载 {}",
        path.display()
    );
    let text = std::fs::read_to_string(path).unwrap();
    let ast = syn::parse_file(&text).unwrap();
    if text.lines().count() >= 500 {
        errors.push(format!("{}: 必须少于500行", path.display()));
    }
    let mut visitor = SourceVisitor::default();
    visitor.visit_file(&ast);
    if visitor.objects > 1 {
        errors.push(format!("{}: {}对象", path.display(), visitor.objects));
    }
    errors.extend(
        visitor
            .errors
            .into_iter()
            .map(|error| format!("{}: {error}", path.display())),
    );
    let base = if path.file_stem().is_some_and(|stem| stem == "mod") {
        path.parent().unwrap().to_owned()
    } else {
        path.with_extension("")
    };
    for item in ast.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if test_only(&module.attrs)
            || module
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("path"))
        {
            continue;
        }
        assert!(module.content.is_none(), "生产模块必须真实分文件");
        let name = module.ident.to_string();
        let direct = base.join(format!("{name}.rs"));
        let child = if direct.exists() {
            direct
        } else {
            base.join(name).join("mod.rs")
        };
        inspect(&child, mounted, errors);
    }
}

#[test]
fn historical_implementations_are_real_documented_responsibility_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut mounted = BTreeSet::new();
    let mut errors = Vec::new();
    for name in ["query.rs", "compare.rs", "historical_node_size.rs"] {
        let path = root.join(name);
        if path.exists() {
            inspect(&path, &mut mounted, &mut errors);
        } else {
            errors.push(format!("缺少真实实现 {name}"));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn gate_rejects_hidden_objects_private_stubs_and_glob_imports() {
    let source = "mod nested { struct A; type B = u8; fn stub() {} use crate::*; fn hidden() { include!(\"elsewhere.rs\"); } }";
    let mut visitor = SourceVisitor::default();
    visitor.visit_file(&syn::parse_file(source).unwrap());
    assert_eq!(visitor.objects, 2);
    for text in ["空实现", "wildcard", "隐藏源"] {
        assert!(
            visitor.errors.iter().any(|error| error.contains(text)),
            "未拒绝{text}"
        );
    }
}

#[test]
fn gate_rejects_production_path_overrides_but_allows_separate_test_sources() {
    let mut production = SourceVisitor::default();
    production
        .visit_file(&syn::parse_file("#[path = \"hidden.rs\"] mod observed_node_size;").unwrap());
    assert!(
        production
            .errors
            .iter()
            .any(|error| error.contains("生产模块路径覆盖")),
        "门禁必须检查真实挂载源，不能允许 path 把实现藏到未检查文件"
    );
    let mut tests = SourceVisitor::default();
    tests.visit_file(
        &syn::parse_file("#[cfg(test)] #[path = \"query_tests.rs\"] mod tests;").unwrap(),
    );
    assert!(tests.errors.is_empty());
}
