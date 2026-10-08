//! 完整单请求校验、授权、分发与末检链；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::business_of;
use crate::error_reply::tool_error;
use crate::protocol::{
    catalog_id_for, initialize_result, protocol_error, served, success_response, tools_list_result,
};
use crate::{McpService, protocol, relation_reply, snapshot_reply, tool_input_schema};
#[cfg(test)]
use crate::{history_budget_tests, relation_budget_tests};
use diskgraph_core::{BusinessError, Envelope, QueryBudget, ScopeId};
use diskgraph_engine::{EngineError, admin_scope};
use serde_json::{Value, json};

impl McpService {
    /// 处理单个已解码请求帧，保留初始化、通知和方法错误语义。
    /// 参数：request 为已解码 JSON-RPC 请求。返回：对应的响应帧。
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

    /// 在传输入口固定的期限内处理请求，不为工具执行重建预算。
    /// 参数：request 为已解码请求，deadline 为原单调期限。返回：关联响应帧。
    pub(crate) fn handle_until(
        &mut self,
        request: &protocol::Request,
        deadline: std::time::Instant,
    ) -> Value {
        if request.method == "tools/call" {
            // 传输剩余额度只可缩短工具原有预算，不能放大默认查询窗口。
            let deadline = match diskgraph_core::query_deadline(QueryBudget::default()) {
                Ok(query_deadline) => deadline.min(query_deadline),
                Err(error) => return tool_error(&request.id, error, &error.to_string()),
            };
            self.call_tool_until(&request.id, &request.params, deadline)
        } else {
            self.handle(request)
        }
    }

    /// 按原期限依次执行工具目录、profile、schema、授权与响应末检。
    /// 参数：id 为请求关联，params 为工具调用字段。返回：成功或协议/业务错误帧。
    fn call_tool(&mut self, id: &Value, params: &Value) -> Value {
        let deadline = match diskgraph_core::query_deadline(QueryBudget::default()) {
            Ok(deadline) => deadline,
            Err(error) => return tool_error(id, error, &error.to_string()),
        };
        self.call_tool_until(id, params, deadline)
    }

    fn call_tool_until(
        &mut self,
        id: &Value,
        params: &Value,
        deadline: std::time::Instant,
    ) -> Value {
        if std::time::Instant::now() >= deadline {
            return tool_error(
                id,
                BusinessError::BudgetExceeded,
                "request deadline exhausted",
            );
        }
        #[cfg(test)]
        relation_budget_tests::request_started();
        #[cfg(test)]
        history_budget_tests::request_started();
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
        match self
            .dispatch(catalog_id, &arguments, deadline)
            .and_then(|data| {
                if matches!(catalog_id, "C13" | "C14" | "C15" | "C16") {
                    relation_reply::finish(self, data, deadline)
                        .map_err(|error| error.with_context(catalog_id))
                } else if matches!(catalog_id, "C06" | "C07") {
                    let mut revisions = [""; 2];
                    for (position, key) in ["before", "after"].into_iter().enumerate() {
                        revisions[position] =
                            arguments.get(key).and_then(Value::as_str).ok_or_else(|| {
                                EngineError::Business(BusinessError::InvalidArgument)
                                    .with_context(catalog_id)
                            })?;
                    }
                    snapshot_reply::finish(self, data, &revisions, deadline)
                        .map_err(|error| error.with_context(catalog_id))
                } else {
                    let text = data.to_string();
                    Ok((data, text))
                }
            }) {
            Ok((data, text)) => success_response(
                id,
                json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": data,
                    "isError": false,
                }),
            ),
            Err(error) => tool_error(id, business_of(&error.error), &error.to_string()),
        }
    }

    /// 逐请求授权并执行目录项，不依赖工具列表是否曾显示该工具。
    /// 参数：catalog_id 为目录项，arguments 为参数，deadline 为原请求期限。返回：业务数据或附目录上下文的错误。
    pub(crate) fn dispatch(
        &mut self,
        catalog_id: &str,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, diskgraph_engine::ContextualEngineError> {
        let outcome = self.dispatch_inner(catalog_id, arguments, deadline);
        match outcome {
            Ok(data) => Ok(data),
            Err(error) => Err(error.with_context(catalog_id)),
        }
    }

    /// 固定真实 revision 所有者和响应身份，再调用相应适配器。
    /// 参数：catalog_id/arguments 为已校验目录请求，deadline 为原期限。返回：实际绑定的业务 Envelope 或错误。
    fn dispatch_inner(
        &mut self,
        catalog_id: &str,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        // 证据 C04 使用实际任务授权和回执身份，不能先用默认 scope 的通用目录权限拒绝。
        if catalog_id == "C04"
            && let Some(envelope) = self.evidence_status_envelope(arguments, deadline)?
        {
            return Ok(envelope);
        }
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
            let authorizer = self.authorizer_until(deadline)?;
            Some(self.engine.authorize_revision_until(
                expected.as_ref(),
                revision,
                self.context.principal(),
                &authorizer,
                deadline,
            )?)
        } else {
            self.resolve_scope_until(arguments, deadline)?
        };
        let mut pinned_arguments = arguments.clone();
        if matches!(catalog_id, "C08" | "C09" | "C10" | "C11" | "C12" | "C16")
            && explicit_revision.is_none()
        {
            let revision = if catalog_id == "C16" {
                self.engine
                    .latest_revision_until(&self.require_scope(&scope)?, deadline)?
                    .ok_or(EngineError::Business(BusinessError::NotIndexed))?
            } else {
                self.require_revision_until(&scope, arguments, deadline)?
            };
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
            self.require_until(&permission, &authorization_scope, deadline)?;
        }
        match catalog_id {
            "C01" => self.scope_tool(arguments),
            "C02" => self.index_tool(&scope, arguments, false),
            "C03" => self.index_tool(&scope, arguments, true),
            "C04" => self.status_tool(arguments),
            "C05" => self.snapshots_tool(&scope, arguments),
            "C06" | "C07" => self.history_tool(catalog_id, arguments, deadline),
            "C08" => self.explore_tool(&scope, arguments),
            "C09" => self.search_tool(&scope, arguments),
            "C10" => self.node_tool(&scope, arguments),
            "C11" => self.children_tool(&scope, arguments),
            "C12" => self.top_tool(&scope, arguments),
            "C13" => self.related_tool(arguments, deadline),
            "C14" => self.explain_tool(arguments, deadline),
            "C15" => self.impact_tool(arguments, deadline),
            "C16" => self.candidates_tool(&scope, arguments, deadline),
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
}
