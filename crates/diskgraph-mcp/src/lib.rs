//! The MCP service: one dispatch table shared by every transport, calling the
//! same engine entry points as the CLI. The transport layer never re-implements
//! query logic and never shells out to the CLI (P3 tasks 4.1–4.4, spec CMD-04).

use std::io::{BufRead, Write};
use std::path::PathBuf;

use diskgraph_core::{
    Authorizer, BusinessError, CursorContext, DiskNode, Envelope, PagingCursor, Permission,
    PolicyAuthorizer, PrincipalId, QueryBudget, Relation, ScopeId, SizeFilter, treemap,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError, admin_scope};
use serde_json::{Value, json};

pub mod auth;
pub mod doctor;
pub mod http;
pub mod install;
pub mod legacy;
pub mod protocol;

use protocol::{
    FrameError, ToolProfile, catalog_id_for, decode_request, initialize_result, log_line,
    protocol_error, served, success_response, tool_error, tools_list_result,
};

/// The principal a stdio server acts as. stdio binds the local user; scope and
/// policy checks still apply, so a tool call cannot widen authorization.
pub const STDIO_PRINCIPAL: &str = "local-user";

/// Server configuration.
#[derive(Clone, Debug)]
pub struct McpConfig {
    pub data_dir: PathBuf,
    pub profile: ToolProfile,
    pub principal: PrincipalId,
    /// Whether the legacy HTTP+SSE adapter is served (default off).
    pub legacy_sse: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("diskgraph-data"),
            profile: ToolProfile::ReadFull,
            principal: PrincipalId::new(STDIO_PRINCIPAL).expect("constant principal is valid"),
            legacy_sse: false,
        }
    }
}

/// The MCP service: an engine, the acting principal, and the tool profile.
pub struct McpService {
    engine: std::sync::Arc<Engine>,
    principal: PrincipalId,
    profile: ToolProfile,
    legacy_sse: bool,
    initialized: bool,
}

impl McpService {
    /// Opens the engine and the local policy, then bootstraps the stdio
    /// principal exactly as the CLI does.
    pub fn open(config: McpConfig) -> Result<Self, EngineError> {
        let engine = Engine::open(EngineConfig {
            data_dir: config.data_dir,
            max_nodes_per_scan: 2_000_000,
            ..EngineConfig::default()
        })?;
        engine.bootstrap_local_admin(&config.principal)?;
        Ok(Self {
            engine: std::sync::Arc::new(engine),
            principal: config.principal,
            profile: config.profile,
            legacy_sse: config.legacy_sse,
            initialized: false,
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Starts a background runner that drains queued jobs to completion,
    /// independent of any client connection (P4 task 5.8, MCP-05). The server
    /// binary keeps the handle alive; tests control it explicitly.
    pub fn start_job_runner(&self) -> diskgraph_engine::JobRunner {
        diskgraph_engine::JobRunner::start(std::sync::Arc::clone(&self.engine))
    }

    /// The live authorizer, rebuilt from the control store so grants issued
    /// after startup (scope registration, policy changes) take effect without
    /// a restart.
    fn authorizer(&self) -> Result<PolicyAuthorizer, EngineError> {
        self.engine.policy_authorizer()
    }

    pub fn profile(&self) -> ToolProfile {
        self.profile
    }

    /// Handles one decoded request frame and returns the response frame, if any.
    pub fn handle(&mut self, request: &protocol::Request) -> Value {
        match request.method.as_str() {
            // The protocol version is echoed from the client's request when it
            // is one we know; otherwise the built-in revision is used.
            "initialize" => {
                self.initialized = true;
                let requested = request
                    .params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(protocol::PROTOCOL_VERSION);
                success_response(&request.id, initialize_result(requested))
            }
            "notifications/initialized" => {
                // Notifications carry no id and expect no response; the frame
                // decoder already filtered them, so this is defensive.
                success_response(&request.id, json!({}))
            }
            "ping" => success_response(&request.id, json!({})),
            "tools/list" => success_response(&request.id, tools_list_result(self.profile)),
            "tools/call" => self.call_tool(&request.id, &request.params),
            other => protocol_error(
                request.id.clone(),
                -32601,
                &format!("unknown method: {other}"),
            ),
        }
    }

    /// Executes one tool call after checking the catalog stage and permissions.
    fn call_tool(&mut self, id: &Value, params: &Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return protocol_error(id.clone(), -32602, "tools/call requires a tool name");
        };
        let Some(catalog_id) = catalog_id_for(name) else {
            return protocol_error(id.clone(), -32601, &format!("unknown tool: {name}"));
        };
        // A tool outside the active profile is not merely hidden: calling it
        // is refused, so a client cannot bypass the profile by naming it.
        if !protocol::tools_for(self.profile)
            .iter()
            .any(|tool| tool.catalog_id == catalog_id)
        {
            return tool_error(
                id,
                BusinessError::Unsupported,
                &format!("{name} is not in the {} profile", self.profile.wire_name()),
            );
        }
        // A family whose stage has not shipped answers `unsupported` rather
        // than disappearing from the list or pretending to succeed.
        if let Err(error) = served(catalog_id) {
            return tool_error(
                id,
                error,
                &format!("{catalog_id} is not served in this build"),
            );
        }
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        match self.dispatch(catalog_id, &arguments) {
            Ok(data) => success_response(
                id,
                json!({
                    "content": [{"type": "text", "text": data.to_string()}],
                    "structuredContent": data,
                    "isError": false,
                }),
            ),
            Err(error) => tool_error(id, business_of(&error.error), &error.to_string()),
        }
    }

    /// Authorizes then runs one catalog entry. Authorization happens here,
    /// per request, independent of what the tool list advertised.
    fn dispatch(
        &mut self,
        catalog_id: &str,
        arguments: &Value,
    ) -> Result<Value, diskgraph_engine::ContextualEngineError> {
        let outcome = self.dispatch_inner(catalog_id, arguments);
        match outcome {
            Ok(data) => Ok(data),
            Err(error) => Err(error.with_context(catalog_id)),
        }
    }

    fn dispatch_inner(
        &mut self,
        catalog_id: &str,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let scope = self.resolve_scope(arguments)?;
        // Scope management is a server-administration capability, so it is
        // checked against the admin scope; everything else against the scope
        // the request actually names.
        // A tool that mixes read and write actions authorizes per action: the
        // read-only `scope list` must not require the write-only
        // `scope:admin` capability its sibling actions need.
        let action = arguments.get("action").and_then(Value::as_str);
        let authorization_scope =
            if catalog_id == "C01" && action != Some("add") && action != Some("remove") {
                scope.clone().unwrap_or_else(admin_scope)
            } else {
                admin_scope()
            };
        for permission in protocol::permissions_for_action(catalog_id, action) {
            self.require(&permission, &authorization_scope)?;
        }
        match catalog_id {
            "C01" => self.scope_tool(arguments),
            "C02" => self.index_tool(&scope, arguments, false),
            "C03" => self.index_tool(&scope, arguments, true),
            "C04" => self.status_tool(arguments),
            "C05" => self.snapshots_tool(&scope, arguments),
            "C06" | "C07" => self.history_tool(catalog_id, arguments),
            "C08" => self.explore_tool(&scope, arguments),
            "C09" => self.search_tool(&scope, arguments),
            "C10" => self.node_tool(&scope),
            "C11" => self.children_tool(&scope, arguments),
            "C12" => self.top_tool(&scope, arguments),
            "C13" => self.related_tool(arguments),
            "C14" => self.explain_tool(arguments),
            "C15" => self.impact_tool(arguments),
            "C16" => self.candidates_tool(&scope, arguments),
            // Any catalog entry without a handler here has not shipped; the
            // business code names that, and the wrapper adds the catalog ID.
            _ => Err(EngineError::Business(BusinessError::Unsupported)),
        }
        .and_then(|data| {
            let revision = scope
                .as_ref()
                .and_then(|scope| self.engine.latest_revision(scope).ok().flatten());
            let revision_id =
                revision.and_then(|revision| diskgraph_core::RevisionId::new(revision).ok());
            Ok(Envelope::ok(data)
                .with_ids(Some(self.engine.server_id()?), scope, revision_id)
                .into_json())
        })
    }

    fn scope_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("list");
        match action {
            "list" => {
                let scopes = self
                    .engine
                    .list_scopes(&self.principal, &self.authorizer()?)?;
                Ok(json!({
                    "scopes": scopes
                        .iter()
                        .map(|scope| json!({
                            "scope_id": scope.scope_id.as_str(),
                            "root_display": scope.root.display(),
                            "revoked": scope.revoked,
                        }))
                        .collect::<Vec<_>>(),
                }))
            }
            // Scope add/remove arrive with the CLI's C28/C01 wiring in P3's
            // install step; MCP exposes listing today.
            _ => Err(EngineError::Business(BusinessError::Unsupported)),
        }
    }

    fn index_tool(
        &self,
        scope: &Option<ScopeId>,
        _arguments: &Value,
        sync: bool,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let job = if sync {
            self.engine
                .sync_scope(&scope_id, &self.principal, &self.authorizer()?)?
        } else {
            self.engine
                .index_scope(&scope_id, &self.principal, &self.authorizer()?)?
        };
        // The job ID is the durable handle a client polls after disconnect
        // (MCP-05): a cancelled connection never loses the business state.
        Ok(json!({
            "job_id": job.job_id,
            "state": "queued",
            "poll_with": "diskgraph_status",
        }))
    }

    fn status_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let Some(job_id) = arguments.get("job_id").and_then(Value::as_str) else {
            // With no job ID the status is service-level.
            return Ok(json!({
                "server_id": self.engine.server_id()?.as_str(),
                "profile": self.profile.wire_name(),
                "tools": protocol::tools_for(self.profile).len(),
                // Capability diagnostics: the legacy adapter ships in this
                // build but serves only when explicitly enabled (P4-5.7).
                "legacy_sse": self.legacy_sse,
            }));
        };
        let record = self.engine.job_status(job_id)?;
        Ok(json!({
            "job_id": record.job_id,
            "scope_id": record.scope_id.as_str(),
            "state": format!("{:?}", record.state).to_ascii_lowercase(),
        }))
    }

    fn snapshots_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20);
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let snapshots = self.engine.list_snapshots(
            &scope_id,
            &self.principal,
            &self.authorizer()?,
            limit,
            offset,
        )?;
        Ok(json!({
            "snapshots": snapshots
                .iter()
                .map(|snapshot| json!({
                    "snapshot_id": snapshot.id,
                    "complete": snapshot.coverage.complete,
                }))
                .collect::<Vec<_>>(),
        }))
    }

    fn history_tool(&self, catalog_id: &str, arguments: &Value) -> Result<Value, EngineError> {
        let before = arguments.get("before").and_then(Value::as_str);
        let after = arguments.get("after").and_then(Value::as_str);
        let (Some(before), Some(after)) = (before, after) else {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        };
        let graph_before = self.engine.load_revision(before)?;
        let graph_after = self.engine.load_revision(after)?;
        if catalog_id == "C07" {
            let growth = graph_after.growth(&graph_before, &graph_before.snapshot.root);
            return Ok(json!({
                "comparable": growth.is_some(),
                "delta_bytes": growth.map(|growth| growth.delta_bytes.to_string()),
            }));
        }
        let report = graph_after.changes(&graph_before);
        Ok(json!({
            "incompatible": report.incompatible.as_ref().map(|reason| {
                diskgraph_engine::incompatibility_name(reason.clone())
            }),
            "added": report
                .changes
                .iter()
                .filter(|change| matches!(change, diskgraph_core::Change::Added { .. }))
                .count(),
            "removed": report
                .changes
                .iter()
                .filter(|change| matches!(change, diskgraph_core::Change::Removed { .. }))
                .count(),
            "size_changed": report
                .changes
                .iter()
                .filter(|change| matches!(change, diskgraph_core::Change::SizeChanged { .. }))
                .count(),
        }))
    }

    fn explore_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope)?;
        let graph = self.engine.load_revision(&revision)?;
        let node_id = arguments
            .get("node_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let summary = diskgraph_engine::explore(&graph, node_id, QueryBudget::default());
        Ok(json!({
            "node": summary.node,
            "children": summary.children,
            "coverage": summary.coverage,
            "truncated": summary.truncated.map(|reason| reason.wire_name()),
        }))
    }

    fn search_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = self.require_revision(scope)?;
        let pattern = arguments
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?;
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
        // The cursor binds the principal, scope, revision, filter, sort, and
        // the policy epoch it was issued under (P4-5.9).
        let pattern_binding = format!("pattern:{pattern}");
        let authorizer = self.authorizer()?;
        let context = CursorContext {
            principal_binding: self.principal.as_str(),
            scope_id: scope_id.as_str(),
            revision_id: &revision,
            filter_binding: &pattern_binding,
            sort_binding: "size_desc,name_asc",
            policy_version: authorizer.policy_version(),
        };
        let start = match arguments.get("cursor").and_then(Value::as_str) {
            Some(encoded) => {
                let decoded = PagingCursor::decode(encoded)
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                decoded
                    .verify(&context)
                    .map_err(|rejection| EngineError::Business(rejection.business_error()))?
            }
            None => offset,
        };
        let graph = self.engine.load_revision(&revision)?;
        let (items, next) =
            diskgraph_engine::search_nodes(&graph, pattern, start, limit, QueryBudget::default());
        let next_cursor = next.map(|offset| {
            PagingCursor::issue(
                &context,
                pattern_binding.clone(),
                "size_desc,name_asc",
                offset,
            )
            .encode()
        });
        Ok(json!({ "items": items, "next_cursor": next_cursor, "next_offset": next }))
    }

    fn node_tool(&self, scope: &Option<ScopeId>) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope)?;
        let graph = self.engine.load_revision(&revision)?;
        let root = graph
            .nodes
            .iter()
            .find(|node| node.parent_id.is_none())
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        Ok(json!({ "node": root, "coverage": graph.snapshot.coverage }))
    }

    /// Turns query results into treemap rows: the same nodes, ordered by
    /// observed size, with the category a collector assigned as the note.
    fn treemap_rows<'a>(items: impl Iterator<Item = &'a DiskNode>) -> Vec<treemap::TextRow> {
        let mut rows: Vec<treemap::TextRow> = items
            .map(|node| treemap::TextRow {
                id: node.id,
                name: node.name.clone(),
                size_bytes: node.subtree_bytes,
                files: node.files,
                note: node.category_hint.clone(),
                muted: node.reclaim_hint.is_none(),
            })
            .collect();
        rows.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.name.cmp(&b.name)));
        rows
    }

    /// Whether the caller asked for the text treemap instead of JSON items.
    /// The tool set does not grow: the same tool renders either shape.
    fn wants_treemap(arguments: &Value) -> bool {
        arguments
            .get("format")
            .and_then(Value::as_str)
            .is_some_and(|format| format == "treemap")
    }

    /// The terminal width the caller wants, defaulting to a readable 88.
    fn treemap_width(arguments: &Value) -> usize {
        arguments.get("width").and_then(Value::as_u64).unwrap_or(88) as usize
    }

    fn children_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope)?;
        let graph = self.engine.load_revision(&revision)?;
        let parent_id = arguments
            .get("parent_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let filter = arguments
            .get("min_bytes")
            .and_then(Value::as_u64)
            .map(SizeFilter::AtLeast);
        let page = graph.children_filtered(parent_id, filter, offset, limit);
        if Self::wants_treemap(arguments) {
            let rows = Self::treemap_rows(page.items.iter().copied());
            let width = Self::treemap_width(arguments);
            return Ok(json!({
                "format": "treemap",
                "treemap": treemap::render_text(&rows, width),
                "items": page.items.len(),
                "next_offset": page.next_offset,
            }));
        }
        Ok(json!({
            "items": page.items,
            "next_offset": page.next_offset,
            "unknown_size_count": page.unknown_count,
        }))
    }

    fn top_tool(&self, scope: &Option<ScopeId>, arguments: &Value) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope)?;
        let graph = self.engine.load_revision(&revision)?;
        let parent_id = arguments
            .get("parent_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
        let items = graph.top(parent_id, limit);
        if Self::wants_treemap(arguments) {
            let rows = Self::treemap_rows(items.iter().copied());
            let width = Self::treemap_width(arguments);
            return Ok(json!({
                "format": "treemap",
                "treemap": treemap::render_text(&rows, width),
                "items": items.len(),
                "size_kind": "allocated",
            }));
        }
        Ok(json!({
            "items": items,
            "size_kind": "allocated",
        }))
    }

    fn related_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        let relation = arguments
            .get("relation")
            .and_then(Value::as_str)
            .map(|name| {
                Relation::parse(name).ok_or(EngineError::Business(BusinessError::InvalidArgument))
            })
            .transpose()?;
        let outgoing = arguments
            .get("direction")
            .and_then(Value::as_str)
            .map(|direction| direction == "outgoing")
            .unwrap_or(true);
        let edges = self.engine.related(
            &revision,
            &entity,
            relation,
            outgoing,
            &self.principal,
            &self.authorizer()?,
        )?;
        Ok(json!({ "edges": edges }))
    }

    fn explain_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        match self.engine.explain_entity(
            &revision,
            &entity,
            &self.principal,
            &self.authorizer()?,
        )? {
            Some((entity, edges, evidence)) => Ok(json!({
                "entity": entity,
                "edges": edges,
                "evidence": evidence,
            })),
            None => Err(EngineError::Business(BusinessError::NotFound)),
        }
    }

    fn impact_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        let edges = self.engine.all_edges(&revision)?;
        let mut by_source: std::collections::HashMap<String, Vec<(String, Relation)>> =
            std::collections::HashMap::new();
        let mut by_target: std::collections::HashMap<String, Vec<(String, Relation)>> =
            std::collections::HashMap::new();
        for edge in edges {
            by_source
                .entry(edge.source_entity_id.clone())
                .or_default()
                .push((edge.target_entity_id.clone(), edge.relation));
            by_target
                .entry(edge.target_entity_id)
                .or_default()
                .push((edge.source_entity_id, edge.relation));
        }
        let entries =
            diskgraph_engine::impact(&by_source, &by_target, &entity, QueryBudget::default())?;
        Ok(json!({
            "entries": entries
                .iter()
                .map(|entry| json!({
                    "entity_id": entry.entity_id,
                    "relation": entry.relation.wire_name(),
                    "depth": entry.depth,
                }))
                .collect::<Vec<_>>(),
            // A read-only impact answer never authorizes a mutation.
            "grants_execution": false,
        }))
    }

    fn candidates_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope)?;
        let graph = self.engine.load_revision(&revision)?;
        let target = arguments
            .get("target_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        Ok(json!({
            "candidates": graph
                .candidates(target)
                .into_iter()
                .map(|candidate| json!({"node": candidate.node, "evidence": candidate.evidence}))
                .collect::<Vec<_>>(),
            "review_only": true,
        }))
    }

    fn revision_and_entity(&self, arguments: &Value) -> Result<(String, String), EngineError> {
        let revision = arguments
            .get("revision")
            .and_then(Value::as_str)
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?
            .to_owned();
        let entity = arguments
            .get("entity")
            .and_then(Value::as_str)
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?
            .to_owned();
        Ok((revision, entity))
    }

    /// The scope argument is optional: without it, the newest non-revoked
    /// scope answers, which keeps simple agent calls short.
    fn resolve_scope(&self, arguments: &Value) -> Result<Option<ScopeId>, EngineError> {
        if let Some(scope) = arguments.get("scope").and_then(Value::as_str) {
            let scope_id = ScopeId::new(scope.to_owned())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // Unknown scopes are not_found, not permission_denied: the caller
            // can tell a wrong scope from a hidden one.
            if self.engine.scope(&scope_id).is_err() {
                return Err(EngineError::Business(BusinessError::NotFound));
            }
            return Ok(Some(scope_id));
        }
        let scopes = self
            .engine
            .list_scopes(&self.principal, &self.authorizer()?)?;
        Ok(scopes
            .into_iter()
            .find(|scope| !scope.revoked)
            .map(|scope| scope.scope_id))
    }

    fn require_scope(&self, scope: &Option<ScopeId>) -> Result<ScopeId, EngineError> {
        scope
            .clone()
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))
    }

    fn require_revision(&self, scope: &Option<ScopeId>) -> Result<String, EngineError> {
        let scope_id = self.require_scope(scope)?;
        self.engine
            .latest_revision(&scope_id)?
            .ok_or(EngineError::Business(BusinessError::NotIndexed))
    }

    fn require(&self, permission: &Permission, scope: &ScopeId) -> Result<(), EngineError> {
        let authorizer = self
            .authorizer()
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
        match authorizer.decide(&self.principal, permission, scope) {
            diskgraph_core::Decision::Allowed => Ok(()),
            diskgraph_core::Decision::Denied(_) => {
                Err(EngineError::Business(BusinessError::PermissionDenied))
            }
        }
    }
}

fn business_of(error: &EngineError) -> BusinessError {
    match error {
        EngineError::Business(business) => *business,
        EngineError::Store(store) => match store {
            diskgraph_store::StoreError::SnapshotNotFound(_)
            | diskgraph_store::StoreError::ScopeNotFound(_)
            | diskgraph_store::StoreError::JobNotFound(_)
            | diskgraph_store::StoreError::RevisionNotFound(_) => BusinessError::NotFound,
            diskgraph_store::StoreError::Conflict(_)
            | diskgraph_store::StoreError::StaleOwner
            | diskgraph_store::StoreError::RetentionViolation(_) => BusinessError::Conflict,
            _ => BusinessError::InternalError,
        },
        EngineError::Io(_) | EngineError::Poisoned => BusinessError::InternalError,
    }
}

/// Reads newline-delimited JSON-RPC frames from `input` and writes responses
/// to `output`. stdout carries protocol only; progress and diagnostics go to
/// the `log` sink (stderr in the binary) (spec MCP-02).
pub fn serve_stdio<R: BufRead, W: Write, L: Write>(
    service: &mut McpService,
    input: R,
    output: &mut W,
    mut log: L,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match decode_request(&line) {
            Ok(request) => {
                let response = service.handle(&request);
                writeln!(
                    output,
                    "{}",
                    serde_json::to_string(&response).unwrap_or_default()
                )?;
                output.flush()?;
            }
            Err(FrameError::Batch) => {
                // Batched frames are refused explicitly rather than partially
                // executed; the id is null because the frame could not be read.
                writeln!(
                    output,
                    "{}",
                    serde_json::to_string(&protocol_error(
                        Value::Null,
                        -32600,
                        "batched requests are not supported; send one frame per line",
                    ))
                    .unwrap_or_default()
                )?;
                output.flush()?;
            }
            Err(error) => {
                let reason = match error {
                    FrameError::Malformed => "malformed",
                    FrameError::MissingMethod => "missing_method",
                    FrameError::Batch => "batch",
                };
                writeln!(log, "{}", log_line("frame_rejected", &[("reason", reason)]))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(profile: ToolProfile, label: &str) -> (McpService, tempfile::TempDir) {
        let directory = tempfile::TempDir::with_prefix(format!("diskgraph-mcp-{label}-")).unwrap();
        let service = McpService::open(McpConfig {
            data_dir: directory.path().join("data"),
            profile,
            principal: PrincipalId::new(STDIO_PRINCIPAL).unwrap(),
            legacy_sse: false,
        })
        .unwrap();
        (service, directory)
    }

    fn call(service: &mut McpService, tool: &str, arguments: Value) -> Value {
        let request = protocol::Request {
            id: json!(1),
            method: "tools/call".to_owned(),
            params: json!({"name": tool, "arguments": arguments}),
        };
        service.handle(&request)
    }

    /// The success envelope: protocol assertions on it, and the business
    /// payload under its `data` key.
    fn structured(response: &Value) -> &Value {
        assert!(
            response["error"].get("data").is_none(),
            "expected success, got: {response}"
        );
        &response["result"]["structuredContent"]
    }

    /// The business payload inside a success envelope.
    fn payload(response: &Value) -> &Value {
        &structured(response)["data"]
    }

    fn seed(service: &mut McpService, root: &std::path::Path) -> String {
        let scope_id = service
            .engine()
            .register_scope(root, &service.principal, &service.authorizer().unwrap())
            .unwrap();
        let job = service
            .engine()
            .index_scope(
                &scope_id,
                &service.principal,
                &service.authorizer().unwrap(),
            )
            .unwrap();
        service.engine().run_job(&job.job_id, "mcp-test").unwrap();
        scope_id.as_str().to_owned()
    }

    fn cargo_project(label: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let workspace =
            tempfile::TempDir::with_prefix(format!("diskgraph-mcp-data-{label}-")).unwrap();
        let root = workspace.path().join("project");
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("target").join("bin"), vec![0; 4096]).unwrap();
        (workspace, root)
    }

    #[test]
    fn initialize_and_tool_listing_work_over_the_protocol() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "list");
        let initialize = service.handle(&protocol::Request {
            id: json!(1),
            method: "initialize".to_owned(),
            params: json!({"protocolVersion": protocol::PROTOCOL_VERSION}),
        });
        assert_eq!(initialize["result"]["serverInfo"]["name"], "diskgraph");
        assert_eq!(
            initialize["result"]["protocolVersion"],
            protocol::PROTOCOL_VERSION
        );

        let listed = service.handle(&protocol::Request {
            id: json!(2),
            method: "tools/list".to_owned(),
            params: json!({}),
        });
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().all(|tool| tool["name"].is_string()));
    }

    #[test]
    fn unknown_methods_and_tools_answer_protocol_errors() {
        let (mut service, _keep) = service(ToolProfile::All, "errors");
        let response = service.handle(&protocol::Request {
            id: json!(9),
            method: "nope".to_owned(),
            params: json!({}),
        });
        assert_eq!(response["error"]["code"], -32601);
        let response = call(&mut service, "diskgraph_nope", json!({}));
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn an_index_job_is_reachable_after_the_call_returns() {
        let (mut service, _keep) = service(ToolProfile::Manage, "job");
        let (project, root) = cargo_project("job");
        let scope = seed(&mut service, &root);

        let response = call(&mut service, "diskgraph_index", json!({"scope": scope}));
        let data = payload(&response);
        let job_id = data["job_id"].as_str().expect("a durable job id");
        assert_eq!(data["state"], "queued");
        assert_eq!(data["poll_with"], "diskgraph_status");

        // A separate call (simulating a reconnect) reports the durable record.
        // The job is created but not run, so the state is exactly what the
        // control store holds.
        let status = call(&mut service, "diskgraph_status", json!({"job_id": job_id}));
        assert_eq!(payload(&status)["state"], "queued");
        assert_eq!(payload(&status)["scope_id"], scope.as_str());
        drop(project);
    }

    #[test]
    fn queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "queries");
        let (project, root) = cargo_project("queries");
        let scope = seed(&mut service, &root);

        let top = call(&mut service, "diskgraph_top", json!({"scope": scope}));
        let envelope = structured(&top);
        assert_eq!(envelope["api_version"], 2);
        assert_eq!(envelope["ok"], true);
        assert!(envelope["server_id"].is_string());
        let data = payload(&top);
        assert!(!data["items"].as_array().unwrap().is_empty());

        // A scope that was never indexed is `not_indexed`, not an empty success.
        let missing = call(
            &mut service,
            "diskgraph_top",
            json!({"scope": "scope-does-not-exist"}),
        );
        assert_eq!(missing["error"]["data"]["business_code"], "not_found");

        // A registered but never indexed scope reports `not_indexed`.
        let empty = project.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let empty_scope = service
            .engine()
            .register_scope(&empty, &service.principal, &service.authorizer().unwrap())
            .unwrap();
        let unindexed = call(
            &mut service,
            "diskgraph_top",
            json!({"scope": empty_scope.as_str()}),
        );
        assert_eq!(unindexed["error"]["data"]["business_code"], "not_indexed");
        drop(project);
    }

    #[test]
    fn explain_returns_typed_evidence_for_a_cargo_project() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "explain");
        let (project, root) = cargo_project("explain");
        let scope = seed(&mut service, &root);
        let revision = service
            .engine()
            .latest_revision(&ScopeId::new(scope.clone()).unwrap())
            .unwrap()
            .unwrap();
        let graph = service.engine().load_revision(&revision).unwrap();
        let target = graph
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .unwrap();

        let response = call(
            &mut service,
            "diskgraph_explain",
            json!({"revision": revision, "entity": format!("resource-{}", target.id)}),
        );
        let data = payload(&response);
        let edges = data["edges"].as_array().unwrap();
        assert!(
            edges
                .iter()
                .any(|edge| edge["relation"] == "owned_by_project")
        );
        assert!(!data["evidence"].as_array().unwrap().is_empty());
        drop(project);
    }

    #[test]
    fn stdio_serves_frames_and_logs_rejects_to_the_log_sink_only() {
        let (mut service, _keep) = service(ToolProfile::ReadMinimal, "stdio");
        let input = "not-json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n";
        let mut output = Vec::new();
        let mut log = Vec::new();
        serve_stdio(&mut service, input.as_bytes(), &mut output, &mut log).unwrap();
        let frames = String::from_utf8(output).unwrap();
        // Exactly one response frame: the malformed line produced no stdout.
        assert_eq!(frames.lines().count(), 1);
        assert!(frames.contains("diskgraph_explore"));
        // The rejection is recorded on the log sink, not on stdout.
        let logged = String::from_utf8(log).unwrap();
        assert!(logged.contains("frame_rejected"));
    }

    #[test]
    fn batched_frames_are_refused_explicitly() {
        let (mut service, _keep) = service(ToolProfile::ReadMinimal, "batch");
        let mut output = Vec::new();
        let mut log = Vec::new();
        serve_stdio(
            &mut service,
            "[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}]\n".as_bytes(),
            &mut output,
            &mut log,
        )
        .unwrap();
        let frames = String::from_utf8(output).unwrap();
        assert!(frames.contains("-32600"));
        assert!(frames.contains("batched requests are not supported"));
    }

    #[test]
    fn a_tool_outside_the_active_profile_is_refused_not_merely_hidden() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "profile-gate");
        // read-full does not advertise scope management, so calling it must
        // fail even though the tool exists in the catalog.
        let listed = service.handle(&protocol::Request {
            id: json!(1),
            method: "tools/list".to_owned(),
            params: json!({}),
        });
        let names: Vec<String> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(!names.contains(&"diskgraph_scope".to_owned()));

        let response = call(&mut service, "diskgraph_scope", json!({"action": "list"}));
        assert_eq!(response["error"]["data"]["business_code"], "unsupported");
    }

    #[test]
    fn the_manage_profile_serves_scope_listing() {
        let (mut service, _keep) = service(ToolProfile::Manage, "manage-scope");
        let (project, root) = cargo_project("manage-scope");
        seed(&mut service, &root);
        let response = call(&mut service, "diskgraph_scope", json!({"action": "list"}));
        let data = payload(&response);
        assert!(!data["scopes"].as_array().unwrap().is_empty());
        drop(project);
    }

    #[test]
    fn two_scopes_stay_isolated_and_reuse_their_own_revisions() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "isolation");
        let (first, first_root) = cargo_project("isolation-a");
        let (second, second_root) = cargo_project("isolation-b");
        let scope_a = seed(&mut service, &first_root);
        let scope_b = seed(&mut service, &second_root);
        assert_ne!(scope_a, scope_b);

        let revision_a = service
            .engine()
            .latest_revision(&ScopeId::new(scope_a.clone()).unwrap())
            .unwrap()
            .unwrap();
        let revision_b = service
            .engine()
            .latest_revision(&ScopeId::new(scope_b.clone()).unwrap())
            .unwrap()
            .unwrap();
        assert_ne!(revision_a, revision_b, "each scope keeps its own revision");

        // A query bound to one scope never returns the other's resources.
        let top_a = call(&mut service, "diskgraph_top", json!({"scope": scope_a}));
        let items_a = payload(&top_a)["items"].as_array().unwrap().len();
        let top_b = call(&mut service, "diskgraph_top", json!({"scope": scope_b}));
        let items_b = payload(&top_b)["items"].as_array().unwrap().len();
        assert!(items_a > 0 && items_b > 0);
        assert_eq!(
            structured(&top_a)["scope_id"],
            scope_a.as_str(),
            "the response must state which scope answered"
        );
        assert_eq!(structured(&top_b)["scope_id"], scope_b.as_str());
        // Each response is bound to its own published revision.
        assert_eq!(structured(&top_a)["revision_id"], revision_a.as_str());
        assert_ne!(
            structured(&top_a)["revision_id"],
            structured(&top_b)["revision_id"]
        );

        // Repeating the query reuses the same revision: no new publication.
        let again = call(&mut service, "diskgraph_top", json!({"scope": scope_a}));
        assert_eq!(payload(&again)["data"], payload(&top_a)["data"]);
        drop(first);
        drop(second);
    }
}

#[cfg(test)]
mod envelope_binding_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn with_ids_serializes_the_scope_and_revision_it_bound() {
        let scope = ScopeId::new("scope-x").unwrap();
        let revision = diskgraph_core::RevisionId::new("rev-y").unwrap();
        let envelope = Envelope::ok(json!({})).with_ids(
            Some(diskgraph_core::ServerId::new("srv").unwrap()),
            Some(scope.clone()),
            Some(revision.clone()),
        );
        let value = envelope.into_json();
        assert_eq!(value["scope_id"], scope.as_str());
        assert_eq!(value["revision_id"], revision.as_str());
    }
}
