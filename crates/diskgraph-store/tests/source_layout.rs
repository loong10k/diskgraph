//! Rust 存储边界门禁：解析生产模块 AST，避免入口和多类型文件再次膨胀。
use std::path::{Path, PathBuf};

use syn::visit::Visit;

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && matches!(&attribute.meta, syn::Meta::List(list) if list.tokens.to_string() == "test")
    })
}

#[derive(Default)]
struct ProductionUseVisitor {
    violations: Vec<String>,
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

impl ProductionUseVisitor {
    fn require_type_doc(&mut self, name: &syn::Ident, attributes: &[syn::Attribute]) {
        let doc = documentation(attributes);
        if !doc.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
            || !(doc.contains("来源：") || doc.contains("对应 Java:"))
        {
            self.violations
                .push(format!("{name}: 缺少中文类型说明与真实来源"));
        }
    }

    fn require_method_doc(
        &mut self,
        name: &syn::Ident,
        attributes: &[syn::Attribute],
        visibility: &syn::Visibility,
        body: &syn::Block,
    ) {
        if matches!(visibility, syn::Visibility::Inherited) {
            return;
        }
        let doc = documentation(attributes);
        if !doc.contains("参数：") || !doc.contains("返回：") {
            self.violations
                .push(format!("{name}: pub 方法缺少中文参数/返回说明"));
        }
        if doc.contains("签名所示") || doc.contains("UNRESOLVED") {
            self.violations
                .push(format!("{name}: 注释必须说明实际结果，不能使用占位说明"));
        }
        if body.stmts.is_empty() {
            self.violations.push(format!("{name}: 禁止空实现"));
        }
    }
}

impl<'ast> Visit<'ast> for ProductionUseVisitor {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.require_type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.require_type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.require_type_doc(&item.ident, &item.attrs);
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.require_method_doc(&item.sig.ident, &item.attrs, &item.vis, &item.block);
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.require_method_doc(&item.sig.ident, &item.attrs, &item.vis, &item.block);
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_use_glob(&mut self, _glob: &'ast syn::UseGlob) {
        self.violations.push("生产代码禁止 wildcard import".into());
    }

    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        if expression.path.is_ident("todo") || expression.path.is_ident("unimplemented") {
            self.violations.push("生产代码禁止占位实现".into());
        }
        syn::visit::visit_macro(self, expression);
    }
}

fn production_modules(path: &Path, out: &mut Vec<(PathBuf, syn::File)>) {
    let file = syn::parse_file(&std::fs::read_to_string(path).unwrap()).unwrap();
    let base = match path.file_stem().unwrap().to_str().unwrap() {
        "lib" | "mod" => path.parent().unwrap().to_path_buf(),
        stem => path.parent().unwrap().join(stem),
    };
    for item in &file.items {
        if let syn::Item::Mod(module) = item
            && module.content.is_none()
            && !test_only(&module.attrs)
        {
            let name = module.ident.to_string();
            let source = base.join(format!("{name}.rs"));
            let source = if source.exists() {
                source
            } else {
                base.join(name).join("mod.rs")
            };
            production_modules(&source, out);
        }
    }
    out.push((path.to_path_buf(), file));
}

#[test]
fn production_storage_entry_types_and_imports_follow_the_rust_contract() {
    let mut files = Vec::new();
    production_modules(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        &mut files,
    );
    let mut violations = Vec::new();
    for (path, file) in files {
        let label = path.file_name().unwrap().to_string_lossy();
        if label == "lib.rs" || label == "mod.rs" {
            for item in &file.items {
                if !matches!(item, syn::Item::Mod(_) | syn::Item::Use(_)) {
                    violations.push(format!("{label}: 入口只允许模块声明及导出"));
                }
            }
        }
        let types = file
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    syn::Item::Struct(_)
                        | syn::Item::Enum(_)
                        | syn::Item::Trait(_)
                        | syn::Item::Type(_)
                )
            })
            .count();
        if types > 1 {
            violations.push(format!("{label}: {types} 个类型共用一个文件"));
        }
        let mut visitor = ProductionUseVisitor::default();
        visitor.visit_file(&file);
        violations.extend(
            visitor
                .violations
                .into_iter()
                .map(|message| format!("{label}: {message}")),
        );
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}
