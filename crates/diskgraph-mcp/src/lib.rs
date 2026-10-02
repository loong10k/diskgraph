//! The MCP service: one dispatch table shared by every transport, calling the
//! same engine entry points as the CLI. The transport layer never re-implements
//! query logic and never shells out to the CLI (P3 tasks 4.1–4.4, spec CMD-04).

use std::io::{BufRead, Write};
use std::path::PathBuf;

use diskgraph_core::{
    Authorizer, BusinessError, CursorContext, DiskNode, Envelope, PagingCursor, Permission,
    PrincipalId, QueryBudget, Relation, ScopeId, treemap,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError, admin_scope};
use serde_json::{Value, json};

pub mod auth;
#[cfg(test)]
mod children_cursor_tests;
pub mod doctor;
pub mod http;
pub mod install;
pub mod legacy;
pub mod protocol;
mod request_authorizer;
mod request_context;
mod sse_slot;
mod tool_input_schema;

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
#[derive(Clone)]
pub struct McpService {
    engine: std::sync::Arc<Engine>,
    context: request_context::RequestContext,
    profile: ToolProfile,
    legacy_sse: bool,
    initialized: bool,
}

impl McpService {
    /// Opens the engine and the local policy, then bootstraps the stdio
    /// principal exactly as the CLI does.
    pub fn open(config: McpConfig) -> Result<Self, EngineError> {
        Self::open_mode(config, true)
    }

    /// 远程服务不创建本地管理员授权。
    pub fn open_remote(config: McpConfig) -> Result<Self, EngineError> {
        Self::open_mode(config, false)
    }

    fn open_mode(config: McpConfig, trusted_local: bool) -> Result<Self, EngineError> {
        let engine = Engine::open(EngineConfig {
            data_dir: config.data_dir,
            max_nodes_per_scan: 2_000_000,
            ..EngineConfig::default()
        })?;
        if trusted_local {
            engine.bootstrap_local_admin(&config.principal)?;
        }
        Ok(Self {
            engine: std::sync::Arc::new(engine),
            context: if trusted_local {
                request_context::RequestContext::local(config.principal)
            } else {
                request_context::RequestContext::unauthenticated(config.principal)
            },
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
    fn authorizer(&self) -> Result<request_authorizer::RequestAuthorizer, EngineError> {
        Ok(request_authorizer::RequestAuthorizer {
            policy: self.engine.policy_authorizer()?,
            capabilities: self.context.capabilities(),
            expires_at: self.context.expires_at(),
        })
    }

    /// 为认证主体创建独立请求状态，共享 Engine，不覆盖本地主体。
    pub fn for_identity(&self, identity: &auth::AuthenticatedPrincipal) -> Self {
        self.for_transport_identity(identity, "http")
    }

    /// 将已认证身份固定到当前传输，请求状态不写入共享 Engine。
    pub(crate) fn for_transport_identity(
        &self,
        identity: &auth::AuthenticatedPrincipal,
        transport: &'static str,
    ) -> Self {
        let mut request = self.clone();
        request.context = request_context::RequestContext::authenticated(identity, transport);
        request
    }

    /// 当前认证主体是否仍有数据库授权，用于终止已撤权的长连接。
    pub(crate) fn identity_is_live(&self, identity: &auth::AuthenticatedPrincipal) -> bool {
        if identity.permissions.is_empty() {
            return false;
        }
        let request = self.for_identity(identity);
        let Ok(policy) = request.authorizer() else {
            return false;
        };
        if identity.permissions.iter().any(|permission| {
            matches!(
                policy.decide(&identity.principal, permission, &admin_scope()),
                diskgraph_core::Decision::Allowed
            )
        }) {
            return true;
        }
        self.engine
            .control_store()
            .ok()
            .and_then(|store| store.list_scopes().ok())
            .is_some_and(|scopes| {
                scopes.iter().any(|scope| {
                    !scope.revoked
                        && identity.permissions.iter().any(|permission| {
                            matches!(
                                policy.decide(&identity.principal, permission, &scope.scope_id),
                                diskgraph_core::Decision::Allowed
                            )
                        })
                })
            })
    }

    /// 请求传输和经过校验的签发方，供日志记录使用。
    pub fn request_transport(&self) -> (&'static str, Option<&str>) {
        (self.context.transport(), self.context.issuer())
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
        if let Err(message) = tool_input_schema::validate_arguments(catalog_id, &arguments) {
            return protocol_error(id.clone(), -32602, &message);
        }
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
        // Relation queries carry an explicit revision. Its persisted owner,
        // rather than the caller's scope hint or default scope, is the only
        // authority and the identity reported in the response envelope.
        let explicit_revision = if matches!(catalog_id, "C06" | "C07") {
            arguments.get("after").and_then(Value::as_str)
        } else if matches!(
            catalog_id,
            "C08" | "C09" | "C10" | "C11" | "C12" | "C13" | "C14" | "C15" | "C16"
        ) {
            arguments.get("revision").and_then(Value::as_str)
        } else {
            None
        };
        let scope = if let Some(revision) = explicit_revision {
            let expected = arguments
                .get("scope")
                .and_then(Value::as_str)
                .map(|scope| {
                    ScopeId::new(scope.to_owned())
                        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))
                })
                .transpose()?;
            Some(self.engine.authorize_revision(
                expected.as_ref(),
                revision,
                self.context.principal(),
                &self.authorizer()?,
            )?)
        } else {
            self.resolve_scope(arguments)?
        };
        let mut pinned_arguments = arguments.clone();
        if matches!(catalog_id, "C08" | "C09" | "C10" | "C11" | "C12" | "C16")
            && explicit_revision.is_none()
        {
            let revision = self.require_revision(&scope, arguments)?;
            pinned_arguments["revision"] = json!(revision);
        }
        let arguments = &pinned_arguments;
        let response_revision = if matches!(catalog_id, "C06" | "C07") {
            arguments.get("after")
        } else {
            arguments.get("revision")
        }
        .and_then(Value::as_str)
        .map(str::to_owned);
        // Scope management is a server-administration capability, so it is
        // checked against the admin scope; everything else against the scope
        // the request actually names.
        // A tool that mixes read and write actions authorizes per action: the
        // read-only `scope list` must not require the write-only
        // `scope:admin` capability its sibling actions need.
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .or_else(|| matches!(catalog_id, "C01" | "C05").then_some("list"));
        let authorization_scope =
            if catalog_id != "C01" || (action != Some("add") && action != Some("remove")) {
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
            "C10" => self.node_tool(&scope, arguments),
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
            let revision = response_revision.or_else(|| {
                scope
                    .as_ref()
                    .and_then(|scope| self.engine.latest_revision(scope).ok().flatten())
            });
            let revision_id =
                revision.and_then(|revision| diskgraph_core::RevisionId::new(revision).ok());
            let truncated = data
                .get("truncated")
                .is_some_and(|value| !matches!(value, Value::Null | Value::Bool(false)));
            let mut envelope =
                Envelope::ok(data).with_ids(Some(self.engine.server_id()?), scope, revision_id);
            envelope.truncated = truncated;
            Ok(envelope.into_json())
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
                    .list_scopes(self.context.principal(), &self.authorizer()?)?;
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
                .sync_scope(&scope_id, self.context.principal(), &self.authorizer()?)?
        } else {
            self.engine
                .index_scope(&scope_id, self.context.principal(), &self.authorizer()?)?
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
        self.require(&Permission::OperationView, &record.scope_id)?;
        if self.engine.scope(&record.scope_id)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
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
            self.context.principal(),
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
        let expected = arguments
            .get("scope")
            .and_then(Value::as_str)
            .map(|scope| {
                ScopeId::new(scope.to_owned())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))
            })
            .transpose()?;
        self.engine.authorize_revision(
            expected.as_ref(),
            before,
            self.context.principal(),
            &self.authorizer()?,
        )?;
        self.engine.authorize_revision(
            expected.as_ref(),
            after,
            self.context.principal(),
            &self.authorizer()?,
        )?;
        if catalog_id == "C07" {
            let growth = self
                .engine
                .growth_between(before, after, std::path::Path::new(""))?;
            return Ok(json!({
                "comparable": growth.is_some(),
                "delta_bytes": growth.map(|growth| growth.delta_bytes.to_string()),
            }));
        }
        self.engine.revision_changes(before, after)
    }

    fn explore_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope, arguments)?;
        let node_id = arguments
            .get("node_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let budget = QueryBudget::default();
        let (node, mut children) =
            self.engine
                .revision_layer(&revision, node_id, budget.max_nodes + 1)?;
        let truncated = (children.len() >= budget.max_nodes).then_some("node_limit");
        children.truncate(budget.max_nodes.saturating_sub(1));
        let base = serde_json::to_vec(&node)
            .map_err(diskgraph_store::StoreError::from)?
            .len();
        if base.saturating_add(1024) > budget.max_response_bytes {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let (children, bytes_truncated) = Self::bound_nodes(children, base)?;
        let truncated = bytes_truncated.or(truncated);
        Ok(json!({
            "node": node,
            "children": children,
            "coverage": self.engine.revision_snapshot(&revision)?.coverage,
            "truncated": truncated,
        }))
    }

    fn search_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = self.require_revision(scope, arguments)?;
        let pattern = arguments
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100);
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
        // The cursor binds the principal, scope, revision, filter, sort, and
        // the policy epoch it was issued under (P4-5.9).
        let pattern_binding = format!("pattern:{pattern}");
        let authorizer = self.authorizer()?;
        let context = CursorContext {
            principal_binding: self.context.principal().as_str(),
            scope_id: scope_id.as_str(),
            revision_id: &revision,
            filter_binding: &pattern_binding,
            sort_binding: "name_asc,id_asc,keyset_v2",
            policy_version: authorizer.policy_version(),
        };
        let cursor = arguments
            .get("cursor")
            .and_then(Value::as_str)
            .map(|encoded| diskgraph_core::SearchCursor::decode(encoded, &context))
            .transpose()
            .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
        let reader = self.engine.revision_reader()?;
        let snapshot = reader.revision(&revision)?.snapshot_id;
        let after = cursor
            .as_ref()
            .map(|cursor| (cursor.last_name.as_str(), cursor.last_id));
        let (items, more) = reader.search_page(&snapshot, pattern, after, offset, limit)?;
        let (items, truncated) = Self::bound_nodes(items, 0)?;
        let more = more || truncated.is_some();
        let consumed = cursor
            .as_ref()
            .map_or(offset, |cursor| cursor.binding.offset)
            .saturating_add(items.len() as u64);
        let next = more.then_some(consumed);
        let next_cursor = items.last().filter(|_| more).map(|last| {
            diskgraph_core::SearchCursor {
                version: 2,
                binding: PagingCursor::issue(
                    &context,
                    pattern_binding.clone(),
                    "name_asc,id_asc,keyset_v2",
                    consumed,
                ),
                last_name: last.name.clone(),
                last_id: last.id,
            }
            .encode()
        });
        Ok(
            json!({ "items": items, "next_cursor": next_cursor, "next_offset": next,"truncated":truncated }),
        )
    }

    /// 将页面编码成本限制在查询响应预算中；超大单节点明确拒绝，避免无进展游标。
    fn bound_nodes(
        mut items: Vec<DiskNode>,
        base_bytes: usize,
    ) -> Result<(Vec<DiskNode>, Option<&'static str>), EngineError> {
        let maximum = QueryBudget::default()
            .max_response_bytes
            .saturating_sub(base_bytes.saturating_add(1024));
        let mut bytes = 0usize;
        let mut kept = 0usize;
        for node in &items {
            let size = serde_json::to_vec(node)
                .map_err(diskgraph_store::StoreError::from)?
                .len();
            if bytes.saturating_add(size) > maximum {
                break;
            }
            bytes += size;
            kept += 1;
        }
        if kept == 0 && !items.is_empty() {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let truncated = (kept < items.len()).then_some("response_byte_limit");
        items.truncate(kept);
        Ok((items, truncated))
    }

    fn node_tool(&self, scope: &Option<ScopeId>, arguments: &Value) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope, arguments)?;
        let node_id = arguments
            .get("node_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let root = self.engine.revision_node(&revision, node_id)?;
        Ok(json!({ "node": root, "coverage": self.engine.revision_snapshot(&revision)?.coverage }))
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
        let scope_id = self.require_scope(scope)?;
        let revision = self.require_revision(scope, arguments)?;
        let parent_id = arguments
            .get("parent_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(100);
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let filter = arguments.get("min_bytes").and_then(Value::as_u64);
        let filter_binding = format!("parent:{parent_id},min_bytes:{}", filter.unwrap_or(0));
        let sort = "subtree_bytes_desc,name_asc,id_asc,children_keyset_v2";
        let authorizer = self.authorizer()?;
        let context = CursorContext {
            principal_binding: self.context.principal().as_str(),
            scope_id: scope_id.as_str(),
            revision_id: &revision,
            filter_binding: &filter_binding,
            sort_binding: sort,
            policy_version: authorizer.policy_version(),
        };
        let cursor = arguments
            .get("cursor")
            .and_then(Value::as_str)
            .map(|encoded| diskgraph_core::ChildrenCursor::decode(encoded, &context))
            .transpose()
            .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
        self.engine.with_authorized_revision_reader(
            &revision, self.context.principal(), &authorizer,
            QueryBudget::default().deadline_ms, |reader, snapshot, _deadline| {
                let after = cursor.as_ref().map(|cursor| (cursor.last_bytes, cursor.last_name.as_str(), cursor.last_id));
                let (items, more, unknown_count) = reader.children_keyset_page(snapshot, parent_id, filter, after, offset, limit)?;
                // 游标本身计入预算；最后一个返回节点决定位置，不能使用 SQL 探针行。
                let (items, truncated) = Self::bound_nodes(items, 16_384)?;
                let more = more || truncated.is_some();
                let consumed = cursor.as_ref().map_or(offset, |cursor| cursor.binding.offset)
                    .saturating_add(items.len() as u64);
                let next_offset = more.then_some(consumed);
                let next_cursor = items.last().filter(|_| more).map(|last| {
                    diskgraph_core::ChildrenCursor {
                        version: 2,
                        binding: PagingCursor::issue(&context, &filter_binding, sort, consumed),
                        last_bytes: last.subtree_bytes,
                        last_name: last.name.clone(),
                        last_id: last.id,
                    }.encode()
                });
                if next_cursor.as_ref().is_some_and(|encoded| encoded.len() > 16_384) {
                    return Err(EngineError::Business(BusinessError::BudgetExceeded));
                }
                let data = if Self::wants_treemap(arguments) {
                    let rows = Self::treemap_rows(items.iter());
                    json!({"format":"treemap","treemap":treemap::render_text(&rows, Self::treemap_width(arguments)),
                        "items":items.len(),"next_offset":next_offset,"next_cursor":next_cursor,
                        "unknown_size_count":unknown_count,"truncated":truncated})
                } else {
                    json!({"items":items,"next_offset":next_offset,"next_cursor":next_cursor,
                        "unknown_size_count":unknown_count,"truncated":truncated})
                };
                if serde_json::to_vec(&data).map_err(diskgraph_store::StoreError::from)?.len()
                    > QueryBudget::default().max_response_bytes.saturating_sub(1024) {
                    return Err(EngineError::Business(BusinessError::BudgetExceeded));
                }
                Ok(data)
            },
        )
    }

    fn top_tool(&self, scope: &Option<ScopeId>, arguments: &Value) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope, arguments)?;
        let parent_id = arguments
            .get("parent_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
        let (_, items) = self
            .engine
            .revision_layer(&revision, parent_id, limit.min(100))?;
        let (items, truncated) = Self::bound_nodes(items, 0)?;
        if Self::wants_treemap(arguments) {
            let rows = Self::treemap_rows(items.iter());
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
            "truncated":truncated,
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
        self.engine.related_bounded(
            &revision,
            &entity,
            relation,
            outgoing,
            arguments.get("after_edge").and_then(Value::as_str),
            arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(QueryBudget::default().max_edges as u64),
            self.context.principal(),
            &self.authorizer()?,
        )
    }

    fn explain_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        self.engine.explain_bounded(
            &revision,
            &entity,
            arguments.get("after_edge").and_then(Value::as_str),
            arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(QueryBudget::default().max_edges as u64),
            self.context.principal(),
            &self.authorizer()?,
        )
    }

    fn impact_tool(&self, arguments: &Value) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        let authorizer = self.authorizer()?;
        let answer = self.engine.revision_impact(
            &revision,
            &entity,
            QueryBudget::default(),
            self.context.principal(),
            &authorizer,
        )?;
        Ok(json!({
            "entries": answer.entries
                .iter()
                .map(|entry| json!({
                    "entity_id": entry.entity_id,
                    "relation": entry.relation.wire_name(),
                    "depth": entry.depth,
                }))
                .collect::<Vec<_>>(),
            "complete": answer.truncated.is_none(),
            "truncated": answer.truncated.map(|reason| reason.wire_name()),
            // A read-only impact answer never authorizes a mutation.
            "grants_execution": false,
        }))
    }

    fn candidates_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision(scope, arguments)?;
        let target = arguments
            .get("target_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let authorizer = self.authorizer()?;
        let answer = self.engine.review_candidates(
            &revision,
            target,
            QueryBudget::default(),
            self.context.principal(),
            &authorizer,
        )?;
        Ok(json!({
            "candidates": answer.candidates
                .into_iter()
                .map(|(node, evidence)| json!({"node": node, "evidence": evidence}))
                .collect::<Vec<_>>(),
            "review_only": true,
            "coverage_complete": answer.coverage_complete,
            "complete": answer.complete,
            "truncated": answer.truncated.map(|reason| reason.wire_name()),
            "selected_bytes": answer.selected_bytes.to_string(),
            "remaining_bytes": answer.remaining_bytes.to_string(),
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
            .list_scopes(self.context.principal(), &self.authorizer()?)?;
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

    fn require_revision(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<String, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = match arguments.get("revision").and_then(Value::as_str) {
            Some(revision) => revision.to_owned(),
            None => self
                .engine
                .latest_revision(&scope_id)?
                .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
        };
        self.engine.authorize_revision(
            Some(&scope_id),
            &revision,
            self.context.principal(),
            &self.authorizer()?,
        )?;
        Ok(revision)
    }

    fn require(&self, permission: &Permission, scope: &ScopeId) -> Result<(), EngineError> {
        let authorizer = self
            .authorizer()
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
        match authorizer.decide(self.context.principal(), permission, scope) {
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
            .register_scope(
                root,
                service.context.principal(),
                &service.authorizer().unwrap(),
            )
            .unwrap();
        let job = service
            .engine()
            .index_scope(
                &scope_id,
                service.context.principal(),
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
    fn tool_calls_reject_unknown_and_mistyped_arguments() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "strict-arguments");
        let (_project, root) = cargo_project("strict-arguments");
        let scope = seed(&mut service, &root);
        for args in [
            json!({"scope":scope,"node_id":"2"}),
            json!({"scope":scope,"node_id":-1}),
            json!({"scope":scope,"node_id":0}),
            json!({"scope":scope,"typo":2}),
            json!({"scope":scope,"revision":null}),
            json!([]),
        ] {
            let response = call(&mut service, "diskgraph_node", args);
            assert_eq!(response["error"]["code"], -32602, "{response}");
        }
    }

    #[test]
    fn explicit_revision_and_node_id_select_historical_data() {
        let (mut service, _keep) = service(ToolProfile::ReadFull, "historical-node");
        let (_project, root) = cargo_project("historical-node");
        let scope = seed(&mut service, &root);
        let scope_id = ScopeId::new(scope.clone()).unwrap();
        let before = service.engine.latest_revision(&scope_id).unwrap().unwrap();
        let graph = service.engine.load_revision(&before).unwrap();
        let target = graph
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .unwrap();
        std::fs::write(root.join("new.txt"), "new revision").unwrap();
        seed(&mut service, &root);
        assert_ne!(
            service.engine.latest_revision(&scope_id).unwrap().unwrap(),
            before
        );
        let node = call(
            &mut service,
            "diskgraph_node",
            json!({"revision":before,"node_id":target.id}),
        );
        assert_eq!(payload(&node)["node"]["id"], target.id, "{node}");
        assert_eq!(structured(&node)["revision_id"], before);
        for tool in [
            "diskgraph_search",
            "diskgraph_top",
            "diskgraph_children",
            "diskgraph_explore",
        ] {
            let mut args = json!({"revision":before});
            if tool == "diskgraph_search" {
                args["pattern"] = json!("new.txt");
            }
            let response = call(&mut service, tool, args);
            assert_eq!(structured(&response)["revision_id"], before, "{response}");
            assert!(
                !payload(&response).to_string().contains("new.txt"),
                "{response}"
            );
        }
        let other = root.parent().unwrap().join("other");
        std::fs::create_dir(&other).unwrap();
        let other_scope = seed(&mut service, &other);
        let rejected = call(
            &mut service,
            "diskgraph_node",
            json!({"scope":other_scope,"revision":before,"node_id":target.id}),
        );
        assert_eq!(
            rejected["error"]["data"]["business_code"],
            "permission_denied"
        );
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
            .register_scope(
                &empty,
                service.context.principal(),
                &service.authorizer().unwrap(),
            )
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
    fn relation_queries_bound_decoding_and_report_continuation() {
        let (mut service, data) = service(ToolProfile::ReadFull, "relation-page");
        let (_project, root) = cargo_project("relation-page");
        let scope = seed(&mut service, &root);
        let revision = service
            .engine
            .latest_revision(&ScopeId::new(scope.clone()).unwrap())
            .unwrap()
            .unwrap();
        let graph = service.engine.load_revision(&revision).unwrap();
        let target = graph
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .unwrap();
        let entity = format!("resource-{}", target.id);
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        let mut edge: Value = serde_json::from_str(
            &db.query_row(
                "SELECT edge_json FROM relations WHERE source_entity_id = ?1 LIMIT 1",
                [&entity],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        )
        .unwrap();
        for n in 0..501 {
            let id = format!("fixture-{n:04}");
            edge["edge_id"] = json!(id);
            db.execute("INSERT INTO relations (snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![graph.snapshot.id,id,entity,edge["target_entity_id"].as_str().unwrap(),edge["relation"].as_str().unwrap(),edge.to_string()]).unwrap();
        }
        // 不属于首屏的损坏记录必须不被解码，页大小限制实际读取工作量。
        db.execute(
            "UPDATE relations SET edge_json = 'invalid JSON' WHERE edge_id = 'fixture-0500'",
            [],
        )
        .unwrap();
        for tool in ["diskgraph_related", "diskgraph_explain"] {
            let response = call(
                &mut service,
                tool,
                json!({"revision":revision,"entity":entity,"limit":1}),
            );
            assert_eq!(
                payload(&response)["edges"].as_array().unwrap().len(),
                1,
                "{response}"
            );
            assert_eq!(structured(&response)["truncated"], true);
            let after = payload(&response)["next_after_edge"].as_str().unwrap();
            let next = call(
                &mut service,
                tool,
                json!({"revision":revision,"entity":entity,"limit":1,"after_edge":after}),
            );
            assert_ne!(
                payload(&response)["edges"][0]["edge_id"],
                payload(&next)["edges"][0]["edge_id"]
            );
            assert!(
                payload(&response).to_string().len() < QueryBudget::default().max_response_bytes
            );
        }
    }

    #[test]
    fn explain_mixed_direction_byte_pages_never_skip_edges() {
        let (mut service, data) = service(ToolProfile::ReadFull, "mixed-page");
        let (_project, root) = cargo_project("mixed-page");
        let scope = seed(&mut service, &root);
        let revision = service
            .engine
            .latest_revision(&ScopeId::new(scope).unwrap())
            .unwrap()
            .unwrap();
        let graph = service.engine.load_revision(&revision).unwrap();
        let target = graph
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .unwrap();
        let entity = format!("resource-{}", target.id);
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        let mut edge: Value = serde_json::from_str(
            &db.query_row(
                "SELECT edge_json FROM relations WHERE source_entity_id=?1 LIMIT 1",
                [&entity],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        )
        .unwrap();
        db.execute(
            "DELETE FROM relations WHERE snapshot_id=?1",
            [&graph.snapshot.id],
        )
        .unwrap();
        edge["evidence_refs"] = json!([]);
        let expected = vec!["a".to_owned(), "b".to_owned(), "z".to_owned()];
        for (n, id) in expected.iter().enumerate() {
            edge["edge_id"] = json!(id);
            edge["source_entity_id"] = json!(if n == 2 {
                "other".to_owned()
            } else {
                entity.clone()
            });
            edge["target_entity_id"] = json!(if n == 2 {
                entity.clone()
            } else {
                "x".repeat(40_000)
            });
            db.execute("INSERT INTO relations (snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![graph.snapshot.id,id,edge["source_entity_id"].as_str(),edge["target_entity_id"].as_str(),edge["relation"].as_str(),edge.to_string()]).unwrap();
        }
        let mut seen = Vec::new();
        let mut after = Value::Null;
        for _ in 0..4 {
            let mut args = json!({"revision":revision,"entity":entity,"limit":100});
            if !after.is_null() {
                args["after_edge"] = after;
            }
            let result = call(&mut service, "diskgraph_explain", args);
            let page = payload(&result);
            seen.extend(
                page["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["edge_id"].as_str().unwrap().to_owned()),
            );
            after = page["next_after_edge"].clone();
            if page["complete"] == true {
                break;
            }
            assert!(!after.is_null(), "a truncated page must advance");
        }
        assert_eq!(seen, expected);
    }

    #[test]
    fn explain_checks_entity_bytes_before_decoding() {
        let (mut service, data) = service(ToolProfile::ReadFull, "entity-budget");
        let (_project, root) = cargo_project("entity-budget");
        let scope = seed(&mut service, &root);
        let revision = service
            .engine
            .latest_revision(&ScopeId::new(scope).unwrap())
            .unwrap()
            .unwrap();
        let snapshot = service.engine.revision_snapshot(&revision).unwrap();
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        // 超预算且损坏的数据应在 serde 分配之前得到预算错误。
        db.execute(
            "UPDATE entities SET entity_json=?1 WHERE snapshot_id=?2",
            rusqlite::params!["x".repeat(100_000), snapshot.id],
        )
        .unwrap();
        let entity: String = db
            .query_row(
                "SELECT entity_id FROM entities WHERE snapshot_id=?1 LIMIT 1",
                [&snapshot.id],
                |row| row.get(0),
            )
            .unwrap();
        let result = call(
            &mut service,
            "diskgraph_explain",
            json!({"revision":revision,"entity":entity}),
        );
        assert_eq!(
            result["error"]["data"]["business_code"], "budget_exceeded",
            "{result}"
        );
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

    #[test]
    fn impact_requires_the_revision_owners_grant_even_when_a_different_scope_is_supplied() {
        use diskgraph_core::{Grant, Permission};

        let (mut service, data) = service(ToolProfile::ReadFull, "impact-scope");
        let (first, first_root) = cargo_project("impact-a");
        let (second, second_root) = cargo_project("impact-b");
        let scope_a = seed(&mut service, &first_root);
        let scope_b = seed(&mut service, &second_root);
        let revision_b = service
            .engine()
            .latest_revision(&ScopeId::new(scope_b.clone()).unwrap())
            .unwrap()
            .unwrap();
        let graph_b = service.engine().load_revision(&revision_b).unwrap();
        let target = graph_b
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .unwrap();
        let auth = auth::Authenticator::new(auth::AuthConfig::single("issuer", "aud", b"test-key"));
        let token = auth::TokenMinter::new(b"test-key").mint(&auth::TokenClaims {
            issuer: "issuer".into(),
            audience: "aud".into(),
            subject: "only-a".into(),
            expires_at_unix_seconds: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 300,
            scope: Some("metadata:read".into()),
        });
        let identity = auth.authenticate(Some(&token)).unwrap();
        let mut control =
            diskgraph_store::ControlStore::open(&data.path().join("data/diskgraph-control.sqlite"))
                .unwrap();
        control
            .upsert_grant(&Grant {
                principal: identity.principal.clone(),
                permission: Permission::MetadataRead,
                scope: ScopeId::new(scope_a.clone()).unwrap(),
                policy_version: control.policy_version().unwrap(),
            })
            .unwrap();
        let mut remote = service.for_identity(&identity);
        let response = call(
            &mut remote,
            "diskgraph_impact",
            json!({"scope":scope_a,"revision":revision_b,"entity":format!("resource-{}", target.id)}),
        );
        assert_eq!(
            response["error"]["data"]["business_code"], "permission_denied",
            "{response}"
        );
        let mismatch = call(
            &mut service,
            "diskgraph_impact",
            json!({"scope":scope_a,"revision":revision_b,"entity":format!("resource-{}",target.id)}),
        );
        assert_eq!(
            mismatch["error"]["data"]["business_code"],
            "permission_denied"
        );
        let authorized = call(
            &mut service,
            "diskgraph_impact",
            json!({"revision":revision_b,"entity":format!("resource-{}",target.id)}),
        );
        assert_eq!(structured(&authorized)["scope_id"], scope_b);
        assert_eq!(structured(&authorized)["revision_id"], revision_b);
        drop(first);
        drop(second);
    }

    #[test]
    fn zero_target_candidates_do_not_decode_the_full_revision() {
        let (mut service, data) = service(ToolProfile::ReadFull, "zero-candidates");
        let (project, root) = cargo_project("zero-candidates");
        let scope = seed(&mut service, &root);
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        db.execute(
            "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
            [],
        )
        .unwrap();
        let response = call(
            &mut service,
            "diskgraph_candidates",
            json!({"scope":scope,"target_bytes":0}),
        );
        assert_eq!(payload(&response)["candidates"], json!([]));
        drop(project);
    }

    #[test]
    fn positive_target_candidates_use_a_narrow_read_and_report_the_target_gap() {
        let (mut service, data) = service(ToolProfile::ReadFull, "positive-candidates");
        let (project, root) = cargo_project("positive-candidates");
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
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        db.execute(
            "INSERT INTO evidence (snapshot_id, node_id, evidence_json) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                graph.snapshot.id,
                i64::try_from(target.id).unwrap(),
                json!({
                    "node_id": target.id, "relation": "rebuildable", "subject": "fixture",
                    "source": "test", "observed_at_unix_ms": 1, "confidence": 100,
                })
                .to_string()
            ],
        )
        .unwrap();
        db.execute(
            "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
            [],
        )
        .unwrap();
        let response = call(
            &mut service,
            "diskgraph_candidates",
            json!({"scope":scope,"target_bytes":u64::MAX}),
        );
        assert_eq!(payload(&response)["review_only"], true);
        assert!(
            payload(&response)["candidates"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        );
        assert_eq!(payload(&response)["complete"], true);
        assert!(
            payload(&response)["remaining_bytes"]
                .as_str()
                .and_then(|bytes| bytes.parse::<u64>().ok())
                .is_some_and(|bytes| bytes > 0)
        );
        drop(project);
    }

    #[test]
    fn incomplete_candidates_do_not_decode_the_full_revision() {
        let (mut service, data) = service(ToolProfile::ReadFull, "incomplete-candidates");
        let (project, root) = cargo_project("incomplete-candidates");
        let scope = seed(&mut service, &root);
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        db.execute(
            "UPDATE snapshots SET snapshot_json = json_set(snapshot_json, '$.coverage.complete', json('false'))",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
            [],
        )
        .unwrap();
        let response = call(
            &mut service,
            "diskgraph_candidates",
            json!({"scope":scope,"target_bytes":1}),
        );
        assert_eq!(payload(&response)["candidates"], json!([]));
        drop(project);
    }

    #[test]
    fn impact_uses_entity_edges_without_decoding_unrelated_relations() {
        let (mut service, data) = service(ToolProfile::ReadFull, "impact-narrow");
        let (project, root) = cargo_project("impact-narrow");
        std::fs::create_dir_all(root.join("other/target")).unwrap();
        std::fs::write(root.join("other/Cargo.toml"), "[package]\nname='other'\n").unwrap();
        std::fs::write(root.join("other/target/bin"), vec![0; 4096]).unwrap();
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
        let db = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
        assert!(
            db.execute(
                "UPDATE relations SET edge_json = 'invalid JSON' WHERE source_entity_id != ?1",
                [format!("resource-{}", target.id)],
            )
            .unwrap()
                > 0
        );
        let response = call(
            &mut service,
            "diskgraph_impact",
            json!({"scope":scope,"revision":revision,"entity":format!("resource-{}",target.id)}),
        );
        let entries = payload(&response)["entries"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry["relation"] == "rebuildable_by"),
            "{response}"
        );
        assert_eq!(payload(&response)["complete"], true);
        drop(project);
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
