//! The C01–C29 command catalog as a shared contract (P0 task 1.5, specs
//! CMD-01 / CMD-02 / CMD-04 / Q-01).
//!
//! CLI, MCP, and FFI must derive their surfaces from this one table instead of
//! reimplementing a private copy. `serve` and `install` are host-configuration
//! entry points and are deliberately not MCP tools (spec CMD-04).

use serde::{Deserialize, Serialize};

use crate::permissions::{FileActionKind, Permission};

/// First delivery stage of the business capability (see docs/command-reference.md).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
    P7,
}

/// One subcommand/action with its own permission requirement.
#[derive(Clone, Copy, Debug)]
pub struct CommandAction {
    pub name: &'static str,
    pub permission: &'static [Permission],
}

/// One catalog entry: a CLI command family, its MCP tool name, permissions,
/// and first delivery stage.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    /// Stable catalog ID, `C01`..`C29`.
    pub id: &'static str,
    /// CLI root word, e.g. `scope`, `children`.
    pub family: &'static str,
    /// Sub-actions (or the single root action) with per-action permissions.
    pub actions: &'static [CommandAction],
    /// Planned stable MCP business tool; `None` marks host-side-only commands.
    pub mcp_tool: Option<&'static str>,
    /// First delivery stage of the business capability.
    pub stage: Stage,
    /// Mutation commands create immutable plans only (CMD-03); `apply` is the sole executor.
    pub plan_only: bool,
    /// Listed in the minimal read profile for agent hosts (design D7).
    pub read_minimal: bool,
    /// Contract notes that tests and docs must keep in sync.
    pub notes: &'static str,
}

const M: Permission = Permission::MetadataRead;
const C: Permission = Permission::ContentRead;
const I: Permission = Permission::IndexWrite;
const S: Permission = Permission::ScopeAdmin;
const O: Permission = Permission::OperationView;

const fn f(kind: FileActionKind) -> Permission {
    Permission::FileAction(kind)
}

const fn one(name: &'static str, permission: &'static [Permission]) -> CommandAction {
    CommandAction { name, permission }
}

/// The complete target catalog. Order and IDs are part of the contract.
pub const CATALOG: &[CommandSpec] = &[
    CommandSpec {
        id: "C01",
        family: "scope",
        actions: &[
            one("add", &[S]),
            // Listing and showing registered scopes is a metadata read: an
            // agent must be able to see which scopes exist to query them.
            one("list", &[M]),
            one("show", &[M]),
            one("remove", &[S]),
        ],
        mcp_tool: Some("diskgraph_scope"),
        stage: Stage::P1,
        plan_only: false,
        read_minimal: false,
        notes: "scope registration never deletes files or recovery records (CMD-05)",
    },
    CommandSpec {
        id: "C02",
        family: "index",
        actions: &[one("index", &[I])],
        mcp_tool: Some("diskgraph_index"),
        stage: Stage::P1,
        plan_only: false,
        read_minimal: false,
        notes: "creates a durable scan job; acceptance is not completion",
    },
    CommandSpec {
        id: "C03",
        family: "sync",
        actions: &[one("sync", &[I])],
        mcp_tool: Some("diskgraph_sync"),
        stage: Stage::P1,
        plan_only: false,
        read_minimal: false,
        notes: "updates a registered scope; merges conflicting jobs",
    },
    CommandSpec {
        id: "C04",
        family: "status",
        actions: &[one("status", &[M, O])],
        mcp_tool: Some("diskgraph_status"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: true,
        notes: "job ownership applies; reports versions, coverage, capabilities",
    },
    CommandSpec {
        id: "C05",
        family: "snapshots",
        actions: &[
            one("list", &[M]),
            one("show", &[M]),
            one("pin", &[I]),
            one("remove", &[I]),
        ],
        mcp_tool: Some("diskgraph_snapshots"),
        stage: Stage::P1,
        plan_only: false,
        read_minimal: false,
        notes: "retention edits graph history only, never user files or control data",
    },
    CommandSpec {
        id: "C06",
        family: "changes",
        actions: &[one("changes", &[M])],
        mcp_tool: Some("diskgraph_changes"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "reports incomparable history with reasons; no rename inference",
    },
    CommandSpec {
        id: "C07",
        family: "growth",
        actions: &[one("growth", &[M])],
        mcp_tool: Some("diskgraph_growth"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "growth is bound to the current revision's comparable history",
    },
    CommandSpec {
        id: "C08",
        family: "explore",
        actions: &[one("explore", &[M])],
        mcp_tool: Some("diskgraph_explore"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: true,
        notes: "bounded directory/relation summary; ambiguity returns candidate IDs",
    },
    CommandSpec {
        id: "C09",
        family: "search",
        actions: &[one("search", &[M])],
        mcp_tool: Some("diskgraph_search"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "name/path/entity patterns only; the server never guesses intent",
    },
    CommandSpec {
        id: "C10",
        family: "node",
        actions: &[one("node", &[M])],
        mcp_tool: Some("diskgraph_node"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "single-node facts with measurement kind and coverage",
    },
    CommandSpec {
        id: "C11",
        family: "children",
        actions: &[one("children", &[M])],
        mcp_tool: Some("diskgraph_children"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "direct children with filters and stable paging",
    },
    CommandSpec {
        id: "C12",
        family: "top",
        actions: &[one("top", &[M])],
        mcp_tool: Some("diskgraph_top"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "largest items with an explicit size kind; unknown sizes stay unknown",
    },
    CommandSpec {
        id: "C13",
        family: "related",
        actions: &[one("related", &[M])],
        mcp_tool: Some("diskgraph_related"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "direction and relation type are mandatory and bounded",
    },
    CommandSpec {
        id: "C14",
        family: "explain",
        actions: &[one("explain", &[M])],
        mcp_tool: Some("diskgraph_explain"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: true,
        notes: "evidence, freshness, conflicts, and unknowns; never generated claims",
    },
    CommandSpec {
        id: "C15",
        family: "impact",
        actions: &[one("impact", &[M])],
        mcp_tool: Some("diskgraph_impact"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "known possible effects only; grants no execution rights",
    },
    CommandSpec {
        id: "C16",
        family: "candidates",
        actions: &[one("candidates", &[M])],
        mcp_tool: Some("diskgraph_candidates"),
        stage: Stage::P2,
        plan_only: false,
        read_minimal: false,
        notes: "eligible_for_review/blocked/unknown; a review queue, not authorization",
    },
    CommandSpec {
        id: "C17",
        family: "duplicates",
        actions: &[one("list", &[M]), one("verify", &[M, C])],
        mcp_tool: Some("diskgraph_duplicates"),
        stage: Stage::P7,
        plan_only: false,
        read_minimal: false,
        notes: "verify needs content permission and a budget; results never auto-delete",
    },
    CommandSpec {
        id: "C18",
        family: "read",
        actions: &[one("read", &[C])],
        mcp_tool: Some("diskgraph_read"),
        stage: Stage::P7,
        plan_only: false,
        read_minimal: false,
        notes: "bounded plain-file byte ranges; never modifies the file",
    },
    CommandSpec {
        id: "C19",
        family: "move",
        actions: &[one("move", &[M, f(FileActionKind::Move)])],
        mcp_tool: Some("diskgraph_move"),
        stage: Stage::P5,
        plan_only: true,
        read_minimal: false,
        notes: "cross-volume moves arrive with P6; execution only via apply",
    },
    CommandSpec {
        id: "C20",
        family: "copy",
        actions: &[one("copy", &[M, f(FileActionKind::Copy)])],
        mcp_tool: Some("diskgraph_copy"),
        stage: Stage::P5,
        plan_only: true,
        read_minimal: false,
        notes: "cross-volume copies arrive with P6; execution only via apply",
    },
    CommandSpec {
        id: "C21",
        family: "trash",
        actions: &[one("trash", &[M, f(FileActionKind::Trash)])],
        mcp_tool: Some("diskgraph_trash"),
        stage: Stage::P5,
        plan_only: true,
        read_minimal: false,
        notes: "recovery plan only; same-volume trash does not reclaim space",
    },
    CommandSpec {
        id: "C22",
        family: "restore",
        actions: &[one("restore", &[O, f(FileActionKind::Restore)])],
        mcp_tool: Some("diskgraph_restore"),
        stage: Stage::P5,
        plan_only: true,
        read_minimal: false,
        notes: "collisions fail or re-plan; originals are never overwritten",
    },
    CommandSpec {
        id: "C23",
        family: "purge",
        actions: &[one("purge", &[C, f(FileActionKind::Purge)])],
        mcp_tool: Some("diskgraph_purge"),
        stage: Stage::P6,
        plan_only: true,
        read_minimal: false,
        notes: "irreversible; independent approval; no restore afterwards",
    },
    CommandSpec {
        id: "C24",
        family: "plan",
        actions: &[
            one("show", &[O]),
            one("validate", &[O]),
            one("create", &[M]),
        ],
        mcp_tool: Some("diskgraph_plan"),
        stage: Stage::P5,
        plan_only: false,
        read_minimal: false,
        notes: "create requires the underlying action permission; validate never mutates",
    },
    CommandSpec {
        id: "C25",
        family: "apply",
        actions: &[one("apply", &[O])],
        mcp_tool: Some("diskgraph_apply"),
        stage: Stage::P5,
        plan_only: false,
        read_minimal: false,
        notes: "needs the action permission plus a trusted approval; no self-approval",
    },
    CommandSpec {
        id: "C26",
        family: "operations",
        actions: &[one("list", &[O]), one("show", &[O]), one("cancel", &[O])],
        mcp_tool: Some("diskgraph_operations"),
        stage: Stage::P5,
        plan_only: false,
        read_minimal: false,
        notes: "cancel stops future steps only; completed items stay completed",
    },
    CommandSpec {
        id: "C27",
        family: "serve",
        actions: &[one("serve", &[S])],
        mcp_tool: None,
        stage: Stage::P3,
        plan_only: false,
        read_minimal: false,
        notes: "stdio in P3, network transports in P4; never a remote MCP tool",
    },
    CommandSpec {
        id: "C28",
        family: "install",
        actions: &[one("add", &[S]), one("show", &[M]), one("remove", &[S])],
        mcp_tool: None,
        stage: Stage::P3,
        plan_only: false,
        read_minimal: false,
        notes: "registers an existing binary into explicit client configs; reversible",
    },
    CommandSpec {
        id: "C29",
        family: "doctor",
        actions: &[one("doctor", &[M])],
        mcp_tool: Some("diskgraph_doctor"),
        stage: Stage::P3,
        plan_only: false,
        read_minimal: false,
        notes: "read-only diagnostics for what the principal may see; no auto-fixes",
    },
];

/// Looks up one catalog entry by its stable ID (`C01`..`C29`).
pub fn by_id(id: &str) -> Option<&'static CommandSpec> {
    CATALOG.iter().find(|spec| spec.id == id)
}

/// Looks up one catalog entry by CLI family name.
pub fn by_family(family: &str) -> Option<&'static CommandSpec> {
    CATALOG.iter().find(|spec| spec.family == family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_holds_exactly_twenty_nine_contiguous_unique_entries() {
        assert_eq!(CATALOG.len(), 29);
        for (index, spec) in CATALOG.iter().enumerate() {
            let expected = format!("C{:02}", index + 1);
            assert_eq!(spec.id, expected, "catalog IDs must stay contiguous");
        }
        let mut families: Vec<_> = CATALOG.iter().map(|spec| spec.family).collect();
        families.sort_unstable();
        families.dedup();
        assert_eq!(families.len(), 29, "CLI families must be unique");
    }

    #[test]
    fn mutation_commands_plan_only_and_apply_is_the_sole_executor() {
        for id in ["C19", "C20", "C21", "C22", "C23"] {
            let spec = by_id(id).unwrap();
            assert!(spec.plan_only, "{id} must be plan-only");
            assert!(
                spec.actions.iter().all(|action| action
                    .permission
                    .iter()
                    .any(|p| matches!(p, Permission::FileAction(_)))),
                "{id} must carry a file-action capability"
            );
        }
        assert!(!by_id("C25").unwrap().plan_only);
    }

    #[test]
    fn read_profile_and_content_permissions_match_the_reference_table() {
        let read_minimal: Vec<_> = CATALOG
            .iter()
            .filter(|spec| spec.read_minimal)
            .map(|spec| spec.id)
            .collect();
        assert_eq!(read_minimal, vec!["C04", "C08", "C14"]);
        assert_eq!(
            by_id("C18").unwrap().actions[0].permission,
            &[Permission::ContentRead]
        );
        let verify = by_id("C17")
            .unwrap()
            .actions
            .iter()
            .find(|action| action.name == "verify")
            .unwrap();
        assert_eq!(verify.permission, &[M, C]);
    }

    #[test]
    fn serve_and_install_are_not_mcp_tools_but_doctor_is() {
        assert!(by_id("C27").unwrap().mcp_tool.is_none());
        assert!(by_id("C28").unwrap().mcp_tool.is_none());
        assert_eq!(by_id("C29").unwrap().mcp_tool, Some("diskgraph_doctor"));
    }

    #[test]
    fn mcp_tool_names_are_unique_across_the_catalog() {
        let mut tools: Vec<_> = CATALOG.iter().filter_map(|spec| spec.mcp_tool).collect();
        tools.sort_unstable();
        tools.dedup();
        assert_eq!(
            tools.len(),
            CATALOG.iter().filter(|s| s.mcp_tool.is_some()).count()
        );
    }

    #[test]
    fn stages_match_the_delivery_plan() {
        let expected: &[(&str, Stage)] = &[
            ("C01", Stage::P1),
            ("C02", Stage::P1),
            ("C03", Stage::P1),
            ("C05", Stage::P1),
            ("C04", Stage::P2),
            ("C16", Stage::P2),
            ("C27", Stage::P3),
            ("C28", Stage::P3),
            ("C29", Stage::P3),
            ("C19", Stage::P5),
            ("C23", Stage::P6),
            ("C17", Stage::P7),
            ("C18", Stage::P7),
        ];
        for (id, stage) in expected {
            assert_eq!(by_id(id).unwrap().stage, *stage, "{id} stage drifted");
        }
    }

    #[test]
    fn every_action_declares_at_least_one_permission() {
        for spec in CATALOG {
            assert!(!spec.actions.is_empty(), "{} has no actions", spec.id);
            for action in spec.actions {
                assert!(!action.permission.is_empty(), "{}.{}", spec.id, action.name);
            }
        }
    }
}
