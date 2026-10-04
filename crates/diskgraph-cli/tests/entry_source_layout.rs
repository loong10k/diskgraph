//! CLI 入口职责门禁。来源：原生 CLI main 的对象分文件与薄入口契约。
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

// 这些既有适配模块不属于本次入口拆分；新增挂载模块全部递归检查。
const EXISTING_ADAPTERS: &[&str] = &[
    "error_reply",
    "html",
    "installer",
    "local",
    "relation_reply",
    "snapshot_action",
    "snapshot_reply",
    "tui",
    "tui_frame_reader",
    "tui_request",
];

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
            // 仅接受实际 rustdoc 条件，保留 Clap 运行时帮助原文；其他 cfg 不算文档。
            if attribute.path().is_ident("cfg_attr") {
                let values = attribute
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                    )
                    .ok()?;
                if values.len() == 2
                    && matches!(values.first(), Some(syn::Meta::Path(path)) if path.is_ident("doc"))
                    && let Some(syn::Meta::NameValue(value)) = values.last()
                    && value.path.is_ident("doc")
                    && let syn::Expr::Lit(value) = &value.value
                    && let syn::Lit::Str(value) = &value.lit
                {
                    return Some(value.value());
                }
                return None;
            }
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

/// 检查实际挂载源中的对象、占位和隐藏实现。来源：CLI 入口结构验收。
#[derive(Default)]
struct SourceVisitor {
    objects: Vec<String>,
    functions: Vec<String>,
    errors: Vec<String>,
}

impl SourceVisitor {
    fn object(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        self.objects.push(name.to_string());
        let text = docs(attributes);
        if !text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            || !(text.contains("来源：") || text.contains("对应 Java:"))
        {
            self.errors
                .push(format!("{name}: 对象缺少中文用途及真实来源"));
        }
    }
}

impl<'ast> Visit<'ast> for SourceVisitor {
    fn visit_item_mod(&mut self, value: &'ast syn::ItemMod) {
        if test_only(&value.attrs) {
            return;
        }
        if value.content.is_some() {
            self.errors.push("生产内联模块隐藏对象职责".into());
        }
        if value
            .attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("path"))
        {
            self.errors.push("生产路径覆盖未按真实标准模块挂载".into());
        }
        syn::visit::visit_item_mod(self, value);
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
        if test_only(&value.attrs) {
            return;
        }
        self.functions.push(value.sig.ident.to_string());
        if value.block.stmts.is_empty() {
            self.errors.push("空函数占位".into());
        }
        syn::visit::visit_item_fn(self, value);
    }
    fn visit_impl_item_fn(&mut self, value: &'ast syn::ImplItemFn) {
        if value.block.stmts.is_empty() {
            self.errors.push("空方法占位".into());
        }
        syn::visit::visit_impl_item_fn(self, value);
    }
    fn visit_use_glob(&mut self, _: &'ast syn::UseGlob) {
        self.errors.push("生产 wildcard import".into());
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

fn inspect(path: &Path, mounted: &mut BTreeSet<PathBuf>, errors: &mut Vec<String>) {
    assert!(
        mounted.insert(path.to_owned()),
        "重复挂载 {}",
        path.display()
    );
    let source = std::fs::read_to_string(path).unwrap();
    let ast = syn::parse_file(&source).unwrap();
    let entry = path.file_name().is_some_and(|name| name == "main.rs");
    let aggregate = path.file_name().is_some_and(|name| name == "mod.rs");
    if source.lines().count() >= 500 {
        errors.push(format!("{}: 实际生产源必须少于500行", path.display()));
    }
    let mut visitor = SourceVisitor::default();
    visitor.visit_file(&ast);
    if (entry || aggregate) && !visitor.objects.is_empty() {
        errors.push(format!(
            "{}: 聚合入口含对象 {:?}",
            path.display(),
            visitor.objects
        ));
    } else if visitor.objects.len() > 1 {
        errors.push(format!(
            "{}: 独立文件含多对象 {:?}",
            path.display(),
            visitor.objects
        ));
    }
    if entry && visitor.functions.iter().any(|name| name != "main") {
        errors.push(format!(
            "{}: main 承担业务方法 {:?}",
            path.display(),
            visitor.functions
        ));
    }
    if aggregate
        && ast
            .items
            .iter()
            .any(|item| !matches!(item, syn::Item::Mod(_) | syn::Item::Use(_)))
    {
        errors.push(format!("{}: mod.rs 只能声明模块和重导出", path.display()));
    }
    errors.extend(
        visitor
            .errors
            .into_iter()
            .map(|error| format!("{}: {error}", path.display())),
    );
    let base = if entry || aggregate {
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
            || module.content.is_some()
        {
            continue;
        }
        let name = module.ident.to_string();
        if entry && EXISTING_ADAPTERS.contains(&name.as_str()) {
            continue;
        }
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
fn cli_entry_and_its_business_modules_have_real_single_object_responsibilities() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let mut mounted = BTreeSet::new();
    let mut errors = Vec::new();
    inspect(&root, &mut mounted, &mut errors);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn gate_rejects_inline_objects_stubs_wildcards_and_hidden_production_paths() {
    let source = "#[path=\"elsewhere.rs\"] mod hidden; mod inline { struct A; type B = u8; fn stub() {} use crate::*; fn hidden() { include!(\"body.rs\"); } }";
    let mut visitor = SourceVisitor::default();
    visitor.visit_file(&syn::parse_file(source).unwrap());
    assert_eq!(visitor.objects, ["A", "B"]);
    for diagnostic in ["内联模块", "路径覆盖", "空函数", "wildcard", "隐藏实现"] {
        assert!(
            visitor
                .errors
                .iter()
                .any(|error| error.contains(diagnostic)),
            "未拒绝{diagnostic}"
        );
    }
}
