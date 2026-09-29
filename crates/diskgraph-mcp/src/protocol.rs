//! MCP protocol layer: JSON-RPC 2.0 framing, the tool catalog derived from
//! the shared command catalog, and tool profiles (P3 tasks 4.1–4.3, specs
//! MCP-02 / MCP-04 / CMD-04).
//!
//! stdout carries protocol frames only; logs and progress go to stderr. Tool
//! exposure is a presentation choice — authorization is enforced per request
//! against the same service entry points the CLI uses.

use diskgraph_core::{BusinessError, CATALOG, CommandSpec, Permission, by_id};
use serde_json::{Value, json};

/// The MCP protocol revision this build speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The implemented server identity reported to clients.
pub const SERVER_NAME: &str = "diskgraph";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which set of tools a client sees (design D7). A profile is a display
/// default, never a capability ceiling: full tools stay callable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolProfile {
    /// The common read tools only (C04/C08/C14).
    ReadMinimal,
    /// Every read tool that needs metadata permission.
    ReadFull,
    /// Scope and index management alongside reads.
    Manage,
    /// Everything enabled in this build (no write tools ship yet).
    All,
}

impl ToolProfile {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "read-minimal" => Self::ReadMinimal,
            "read-full" => Self::ReadFull,
            "manage" => Self::Manage,
            "all" => Self::All,
            _ => return None,
        })
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::ReadMinimal => "read-minimal",
            Self::ReadFull => "read-full",
            Self::Manage => "manage",
            Self::All => "all",
        }
    }

    /// Whether a catalog entry is visible under this profile. `serve` and
    /// `install` are host-side only and never appear (CMD-04).
    fn shows(self, spec: &CommandSpec) -> bool {
        let Some(tool) = spec.mcp_tool else {
            return false;
        };
        let _ = tool;
        match self {
            Self::ReadMinimal => spec.read_minimal,
            Self::ReadFull => spec.read_minimal || is_read_query(spec),
            Self::Manage => {
                spec.read_minimal
                    || is_read_query(spec)
                    || matches!(spec.id, "C01" | "C02" | "C03" | "C05")
            }
            Self::All => true,
        }
    }
}

/// Read-only query families that need only metadata permission.
fn is_read_query(spec: &CommandSpec) -> bool {
    matches!(
        spec.id,
        "C04"
            | "C05"
            | "C06"
            | "C07"
            | "C08"
            | "C09"
            | "C10"
            | "C11"
            | "C12"
            | "C13"
            | "C14"
            | "C15"
            | "C16"
    )
}

/// One tool as advertised over MCP: its stable business name, description,
/// and the catalog entry it implements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDescriptor {
    pub catalog_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub actions: Vec<&'static str>,
    pub required_permissions: Vec<String>,
}

impl ToolDescriptor {
    /// The MCP `tools/list` entry for this tool.
    pub fn schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "description": "Scope ID from diskgraph_scope"},
                    "revision": {"type": "string", "description": "Published graph revision ID"},
                    "limit": {"type": "integer", "minimum": 1},
                    "cursor": {"type": "string", "description": "Opaque paging cursor"},
                },
                "additionalProperties": false,
            },
        })
    }
}

/// The tools visible in one profile, in stable catalog order.
pub fn tools_for(profile: ToolProfile) -> Vec<ToolDescriptor> {
    CATALOG
        .iter()
        .filter(|spec| profile.shows(spec))
        .filter_map(descriptor)
        .collect()
}

/// Builds a descriptor from a catalog entry.
pub fn descriptor(spec: &CommandSpec) -> Option<ToolDescriptor> {
    let name = spec.mcp_tool?;
    let mut permissions: Vec<String> = Vec::new();
    let mut actions = Vec::new();
    for action in spec.actions {
        actions.push(action.name);
        for permission in action.permission {
            let wire = permission.wire_name();
            if !permissions.contains(&wire) {
                permissions.push(wire);
            }
        }
    }
    permissions.sort();
    Some(ToolDescriptor {
        catalog_id: spec.id,
        name,
        description: spec.notes,
        actions,
        required_permissions: permissions,
    })
}

/// Resolves a tool name back to its catalog entry.
pub fn catalog_id_for(tool_name: &str) -> Option<&'static str> {
    CATALOG
        .iter()
        .find(|spec| spec.mcp_tool == Some(tool_name))
        .map(|spec| spec.id)
}

/// Whether this build serves the given catalog entry. Host-side commands and
/// undelivered stages answer `unsupported` rather than disappearing silently.
pub fn served(catalog_id: &str) -> Result<&'static CommandSpec, BusinessError> {
    let spec = by_id(catalog_id).ok_or(BusinessError::NotFound)?;
    match spec.stage {
        diskgraph_core::Stage::P1 | diskgraph_core::Stage::P2 | diskgraph_core::Stage::P3 => {
            Ok(spec)
        }
        _ => Err(BusinessError::Unsupported),
    }
}

/// Builds the `initialize` result.
pub fn initialize_result(protocol_version: &str) -> Value {
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
    })
}

/// Builds the `tools/list` result for a profile.
pub fn tools_list_result(profile: ToolProfile) -> Value {
    json!({ "tools": tools_for(profile).iter().map(ToolDescriptor::schema).collect::<Vec<_>>() })
}

/// A decoded JSON-RPC request. Batch requests are not accepted: one frame,
/// one request, so a malformed line can never be silently half-executed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

/// Why a line is not a valid request frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    Malformed,
    Batch,
    MissingMethod,
}

/// Decodes one JSON-RPC 2.0 request frame.
pub fn decode_request(line: &str) -> Result<Request, FrameError> {
    let value: Value = serde_json::from_str(line).map_err(|_| FrameError::Malformed)?;
    if value.is_array() {
        return Err(FrameError::Batch);
    }
    let Some(object) = value.as_object() else {
        return Err(FrameError::Malformed);
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(FrameError::Malformed);
    }
    let id = object.get("id").cloned().ok_or(FrameError::Malformed)?;
    if id.is_null() {
        // A notification has no id and expects no response.
        return Err(FrameError::Malformed);
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or(FrameError::MissingMethod)?
        .to_owned();
    let params = object.get("params").cloned().unwrap_or(json!({}));
    Ok(Request { id, method, params })
}

/// Builds a success response frame.
pub fn success_response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Builds a JSON-RPC protocol error (malformed frame, unknown method).
pub fn protocol_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Builds a tool error carrying the business result (protocol errors and
/// business refusals stay distinguishable, per CMD-02).
pub fn tool_error(id: &Value, error: BusinessError, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32001,
            "message": message,
            "data": {
                "business_code": error.code(),
                "exit_code": error.exit_code(),
            },
        },
    })
}

/// Structured-log line for stderr; never written to stdout.
pub fn log_line(event: &str, detail: &[(&str, &str)]) -> String {
    let mut payload = json!({"event": event});
    for (key, value) in detail {
        payload[*key] = json!(value);
    }
    payload.to_string()
}

/// The permissions a tool call needs before the service runs it, keyed by
/// tool name.
pub fn required_permissions(tool_name: &str) -> Vec<Permission> {
    let Some(catalog_id) = catalog_id_for(tool_name) else {
        return Vec::new();
    };
    required_permissions_for(catalog_id)
}

/// The permissions one catalog entry needs, deduplicated and stable.
pub fn required_permissions_for(catalog_id: &str) -> Vec<Permission> {
    permissions_for_action(catalog_id, None)
}

/// The permissions for one action of a catalog entry, or the union across all
/// its actions when `action` is None. A read-only action must not inherit the
/// write capability of a sibling action in the same family.
pub fn permissions_for_action(catalog_id: &str, action: Option<&str>) -> Vec<Permission> {
    let Some(spec) = by_id(catalog_id) else {
        return Vec::new();
    };
    let mut permissions: Vec<Permission> = Vec::new();
    let selected = spec.actions.iter().filter(|candidate| match action {
        Some(name) => candidate.name == name,
        None => true,
    });
    for entry in selected {
        for permission in entry.permission {
            if !permissions.contains(permission) {
                permissions.push(*permission);
            }
        }
    }
    permissions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_minimal_profile_exposes_exactly_the_common_read_tools() {
        let names: Vec<&str> = tools_for(ToolProfile::ReadMinimal)
            .iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            names,
            vec!["diskgraph_status", "diskgraph_explore", "diskgraph_explain"]
        );
    }

    #[test]
    fn manage_adds_scope_and_index_management() {
        let names: Vec<&str> = tools_for(ToolProfile::Manage)
            .iter()
            .map(|tool| tool.name)
            .collect();
        for expected in ["diskgraph_scope", "diskgraph_index", "diskgraph_sync"] {
            assert!(names.contains(&expected), "{expected} missing from manage");
        }
    }

    #[test]
    fn read_full_adds_every_metadata_query_but_no_write_tools() {
        let names: Vec<&str> = tools_for(ToolProfile::ReadFull)
            .iter()
            .map(|tool| tool.name)
            .collect();
        for expected in [
            "diskgraph_node",
            "diskgraph_children",
            "diskgraph_top",
            "diskgraph_search",
            "diskgraph_related",
            "diskgraph_impact",
            "diskgraph_candidates",
            "diskgraph_changes",
            "diskgraph_growth",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from read-full"
            );
        }
        // No file action or governance tool is ever advertised in P3.
        // Scope and index management belong to the manage profile only.
        for managed in ["diskgraph_scope", "diskgraph_index", "diskgraph_sync"] {
            assert!(
                !names.contains(&managed),
                "{managed} must not be in read-full"
            );
        }
        for forbidden in [
            "diskgraph_move",
            "diskgraph_trash",
            "diskgraph_purge",
            "diskgraph_apply",
            "diskgraph_plan",
            "diskgraph_operations",
        ] {
            assert!(
                !names.contains(&forbidden),
                "{forbidden} must not be exposed"
            );
        }
    }

    #[test]
    fn host_side_commands_are_never_mcp_tools() {
        for profile in [
            ToolProfile::ReadMinimal,
            ToolProfile::ReadFull,
            ToolProfile::Manage,
            ToolProfile::All,
        ] {
            let names: Vec<&str> = tools_for(profile).iter().map(|tool| tool.name).collect();
            assert!(!names.iter().any(|name| name.contains("serve")));
            assert!(!names.iter().any(|name| name.contains("install")));
        }
        assert!(by_id("C27").unwrap().mcp_tool.is_none());
        assert!(by_id("C28").unwrap().mcp_tool.is_none());
    }

    #[test]
    fn tool_names_are_unique_and_map_back_to_their_catalog_id() {
        let tools = tools_for(ToolProfile::All);
        let mut names: Vec<&str> = tools.iter().map(|tool| tool.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "tool names must be unique");
        for tool in &tools {
            assert_eq!(catalog_id_for(tool.name), Some(tool.catalog_id));
        }
    }

    #[test]
    fn schemas_declare_their_permissions_and_reject_unknown_properties() {
        let schema = descriptor(by_id("C16").unwrap()).unwrap().schema();
        let input = &schema["inputSchema"];
        assert_eq!(input["additionalProperties"], false);
        assert!(input["properties"]["scope"].is_object());
        let descriptor = descriptor(by_id("C21").unwrap()).unwrap();
        // Trash requires both metadata read and its own file action.
        assert!(
            descriptor
                .required_permissions
                .contains(&"metadata:read".to_owned())
        );
        assert!(
            descriptor
                .required_permissions
                .contains(&"files:trash".to_owned())
        );
    }

    #[test]
    fn decoding_rejects_malformed_batched_and_notification_frames() {
        assert_eq!(decode_request("not json"), Err(FrameError::Malformed));
        assert_eq!(decode_request("[]"), Err(FrameError::Batch));
        assert_eq!(
            decode_request("{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"x\"}"),
            Err(FrameError::Malformed)
        );
        assert_eq!(
            decode_request("{\"jsonrpc\":\"2.0\",\"method\":\"x\"}"),
            Err(FrameError::Malformed)
        );
        assert_eq!(
            decode_request("{\"jsonrpc\":\"2.0\",\"id\":1}"),
            Err(FrameError::MissingMethod)
        );
        let request =
            decode_request("{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}").unwrap();
        assert_eq!(request.method, "tools/list");
        assert_eq!(request.id, json!(7));
    }

    #[test]
    fn undelivered_stages_answer_unsupported_not_silence() {
        // C23 purge is a P6 family: it exists, and it says so.
        assert_eq!(served("C23").unwrap_err(), BusinessError::Unsupported);
        assert_eq!(served("C01").unwrap().id, "C01");
        assert_eq!(served("C99").unwrap_err(), BusinessError::NotFound);
    }

    #[test]
    fn business_failures_carry_their_code_and_exit_code_over_mcp() {
        let frame = tool_error(&json!(3), BusinessError::NotIndexed, "scope not indexed");
        assert_eq!(frame["error"]["data"]["business_code"], "not_indexed");
        assert_eq!(frame["error"]["data"]["exit_code"], 4);
        // A protocol error stays distinct from a business refusal.
        let protocol = protocol_error(json!(3), -32601, "unknown method");
        assert!(protocol["error"].get("data").is_none());
    }

    #[test]
    fn required_permissions_never_include_approval_capabilities() {
        // There is no "approve" permission in any tool's requirement set.
        for tool in tools_for(ToolProfile::All) {
            assert!(
                !tool
                    .required_permissions
                    .iter()
                    .any(|name| name.contains("approv")),
                "{} must not require an approval permission",
                tool.name
            );
        }
    }
}
