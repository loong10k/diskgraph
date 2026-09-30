//! Single-file HTML treemap: the browser surface.
//!
//! The page embeds its own layout and rendering code and references no
//! external asset or CDN, so a report opens from a file:// URL on a
//! machine with no network - the same local-first promise the rest of
//! DiskGraph makes. The data is the tree `core` already produced; this
//! module chooses the colours, injects the payload, and says so when the
//! view is a depth-limited slice rather than the whole tree.

use serde_json::Value;

use diskgraph_core::treemap;

/// The palette. A named category gets a stable hue; anything uncategorised
/// stays neutral so a real category always reads as meaningful.
pub const PALETTE: &[(&str, &str)] = &[
    ("code", "#3f6ea8"),
    ("documents", "#7a6ea8"),
    ("media", "#a86ea0"),
    ("git", "#c46a7a"),
    ("toolchains", "#5a9e8f"),
    ("cache", "#c9a227"),
    ("agent", "#8a6a4a"),
    ("build", "#4a8a6a"),
    ("", "#4a5462"),
];

const TEMPLATE: &str = include_str!("assets/treemap.html");

/// Picks a colour for a category hint, defaulting to the neutral band.
/// Shared with the terminal surface, which paints from the same palette.
#[allow(dead_code, reason = "the terminal surface paints from this palette")]
pub fn color_for(category: Option<&str>) -> &'static str {
    let neutral = PALETTE[PALETTE.len() - 1].1;
    let Some(category) = category else {
        return neutral;
    };
    let lowered = category.to_lowercase();
    PALETTE
        .iter()
        .find(|(name, _)| !name.is_empty() && lowered.contains(name))
        .map_or(neutral, |(_, color)| *color)
}

/// Whether any node in the tree reports itself cut off. The page says so
/// rather than letting a slice read as the whole disk.
pub fn tree_is_truncated(tree: &Value) -> bool {
    fn walk(node: &Value) -> bool {
        if node.get("truncated").and_then(Value::as_bool) == Some(true) {
            return true;
        }
        node.get("children")
            .and_then(Value::as_array)
            .is_some_and(|kids| kids.iter().any(walk))
    }
    walk(tree)
}

/// Renders a complete, self-contained page for one tree.
///
/// `expand_depth` is how many levels the page subdivides before it stops
/// subdividing for legibility; the data still carries whatever the caller
/// rendered.
pub fn render_page(
    tree: &Value,
    root: &str,
    revision: &str,
    truncated: bool,
    expand_depth: usize,
) -> String {
    // A cut deep inside the tree is a normal detail, not a warning about the
    // whole report; only a root-level cut changes what the map claims to be.
    let root_cut = tree
        .get("children")
        .and_then(Value::as_array)
        .is_some_and(|kids| {
            kids.iter()
                .any(|kid| kid.get("truncated").and_then(Value::as_bool) == Some(true))
        });
    let notice = match (truncated, root_cut) {
        (_, true) => {
            "<div class=\"notice\">This view is depth-limited: deeper children are not shown. Re-run with a larger --depth or a smaller --min-bytes.</div>"
        }
        (true, false) => {
            "<div class=\"notice faint\">Some branches are cut at the requested depth; the areas shown are exact.</div>"
        }
        (false, false) => "",
    };
    let payload = serde_json::json!({
        "root": root,
        "revision": revision,
        "tree": tree,
    });
    let palette: Vec<[&str; 2]> = PALETTE
        .iter()
        .map(|(name, color)| [*name, *color])
        .collect();
    TEMPLATE
        .replace("__TITLE__", root)
        .replace("__NOTICE__", notice)
        .replace("__DATA__", &payload.to_string())
        .replace(
            "__PALETTE__",
            &serde_json::to_string(&palette).unwrap_or_default(),
        )
        .replace("__EXPAND__", &expand_depth.clamp(1, 5).to_string())
}

/// Rewrites every name in a rendered tree to a stable pseudonym
/// (`home`, `dir-01`, `dir-02`, ...), so a report can be shared without
/// disclosing real project or user names. Sizes, structure, kinds and
/// categories are preserved - they are the point of the report - while
/// everything identifying is not.
///
/// The root path and revision id are likewise replaced by the caller's
/// share-safe labels.
pub fn anonymize_tree(tree: &mut Value, root_label: &str) {
    let mut counter = 0_usize;
    rename_names(tree, root_label, &mut counter, true);
}

fn rename_names(node: &mut Value, root_label: &str, counter: &mut usize, is_root: bool) {
    if is_root {
        if let Some(name) = node.get_mut("name") {
            *name = Value::String(root_label.to_owned());
        }
    } else if let Some(name) = node.get_mut("name") {
        *name = Value::String(next_pseudonym(counter));
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children.iter_mut() {
            rename_names(child, root_label, counter, false);
        }
    }
}

/// The next stable pseudonym (`dir-01`, `dir-02`, ...) from a shared counter,
/// so the terminal surface can anonymize layer by layer and agree with the
/// HTML surface's naming.
pub fn next_pseudonym(counter: &mut usize) -> String {
    *counter += 1;
    format!("dir-{counter:02}")
}

/// A plain-text treemap of one layer, for terminals and agents.
#[allow(dead_code, reason = "the terminal surface renders through here")]
pub fn render_text_map(rows: &[treemap::TextRow], width: usize) -> String {
    treemap::render_text(rows, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "name": "root",
            "kind": "directory",
            "size_bytes": 3000,
            "own_bytes": 0,
            "files": 30,
            "dirs": 3,
            "children": [
                {"name": "code", "kind": "directory", "size_bytes": 2000, "own_bytes": 0,
                 "files": 20, "dirs": 2, "category_hint": "code"},
                {"name": "cache", "kind": "directory", "size_bytes": 1000, "own_bytes": 0,
                 "files": 10, "dirs": 1, "category_hint": "cache"}
            ]
        })
    }

    #[test]
    fn the_page_is_self_contained_and_works_offline() {
        let page = render_page(&sample(), "/tmp/demo", "rev-1", false, 3);
        assert!(page.starts_with("<!DOCTYPE html>"));
        // Not one external reference: the promise is that a report opens on
        // a machine with no network.
        for forbidden in ["http://", "https://", "//cdn", "src=\"http", "integrity="] {
            assert!(
                !page.contains(forbidden),
                "the page must not reference {forbidden}"
            );
        }
        assert!(page.contains("\"revision\":\"rev-1\""));
    }

    #[test]
    fn every_placeholder_is_filled() {
        let page = render_page(&sample(), "/tmp/demo", "rev-1", true, 2);
        for placeholder in [
            "__DATA__",
            "__PALETTE__",
            "__EXPAND__",
            "__TITLE__",
            "__NOTICE__",
        ] {
            assert!(
                !page.contains(placeholder),
                "{placeholder} was never filled"
            );
        }
        assert!(page.contains("EXPAND_DEPTH = 2"));
        // A deep-only cut is a faint note; the loud banner is for a cut at
        // the top level, which changes what the map claims to be.
        assert!(page.contains("notice faint"));
    }

    #[test]
    fn a_root_level_cut_gets_the_full_banner() {
        let mut tree = sample();
        tree["children"][0]["truncated"] = json!(true);
        let page = render_page(&tree, "/tmp/demo", "rev-1", true, 2);
        assert!(page.contains("depth-limited"));
        assert!(!page.contains("notice faint"));
    }

    #[test]
    fn truncation_is_read_from_the_tree_not_guessed_from_the_depth() {
        let mut deep = sample();
        deep["children"][0]["truncated"] = json!(true);
        assert!(tree_is_truncated(&deep));
        assert!(!tree_is_truncated(&sample()));
        let shallow = json!({"name": "x", "children": []});
        assert!(!tree_is_truncated(&shallow));
    }

    #[test]
    fn categories_get_stable_colours_and_the_rest_stay_neutral() {
        assert_eq!(color_for(Some("code")), "#3f6ea8");
        assert_eq!(color_for(Some("Code")), "#3f6ea8");
        assert_eq!(color_for(Some("build output")), "#4a8a6a");
        assert_eq!(color_for(None), "#4a5462");
        assert_eq!(color_for(Some("something else entirely")), "#4a5462");
    }

    #[test]
    fn the_page_carries_the_tree_verbatim() {
        let page = render_page(&sample(), "/tmp/demo", "rev-abc", false, 2);
        assert!(page.contains("\"name\":\"code\""));
        assert!(page.contains("\"size_bytes\":2000"));
    }
}

#[cfg(test)]
mod anonymize_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_become_pseudonyms_but_sizes_survive() {
        let mut tree = json!({
            "name": "wandl",
            "kind": "directory",
            "size_bytes": 214_000_000_000u64,
            "own_bytes": 0,
            "files": 3_600_000,
            "dirs": 679_000,
            "children": [
                {"name": "workspace-tianyin", "kind": "directory", "size_bytes": 2_000_000_000u64,
                 "own_bytes": 0, "files": 2, "dirs": 1},
                {"name": "Library", "kind": "directory", "size_bytes": 48_000_000_000u64,
                 "own_bytes": 0, "files": 3, "dirs": 1}
            ]
        });
        anonymize_tree(&mut tree, "home");
        assert_eq!(tree["name"], "home");
        assert_eq!(tree["children"][0]["name"], "dir-01");
        assert_eq!(tree["children"][1]["name"], "dir-02");
        // Sizes are the point of the report; only names are redacted.
        assert_eq!(tree["size_bytes"], 214_000_000_000u64);
        assert_eq!(tree["children"][0]["size_bytes"], 2_000_000_000u64);
    }

    #[test]
    fn pseudonyms_are_stable_across_calls() {
        let mut first = json!({"name": "secret-project", "kind": "directory"});
        let mut second = json!({"name": "another-project", "kind": "directory"});
        anonymize_tree(&mut first, "home");
        anonymize_tree(&mut second, "home");
        assert_eq!(first["name"], "home");
        assert_eq!(second["name"], "home");
    }
}
