//! 文件系统查询、游标与响应编码预算；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::McpService;
use diskgraph_core::{
    Authorizer, BusinessError, CursorContext, DiskNode, PagingCursor, QueryBudget, ScopeId, treemap,
};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

impl McpService {
    /// 读取指定节点及受节点数、编码字节限制的子层。
    /// 参数：scope 为范围断言，deadline 为原请求期限，arguments 为 revision/node 字段。返回：节点、子层、覆盖率与截断信息或错误。
    pub(crate) fn explore_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision_until(scope, arguments, deadline)?;
        let authorizer = self.authorizer_until(deadline)?;
        self.engine.with_authorized_revision_reader_until(
            &revision,
            self.context.principal(),
            &authorizer,
            deadline,
            |reader, snapshot, _deadline| {
                let node_id = arguments
                    .get("node_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let budget = QueryBudget::default();
                let node = reader
                    .node(snapshot, node_id)?
                    .ok_or(BusinessError::NotFound)?;
                let mut children =
                    reader.children(snapshot, node_id, 0, (budget.max_nodes + 1) as u64)?;
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
                    "coverage": reader.snapshot(snapshot)?.coverage,
                    "truncated": truncated,
                }))
            },
        )
    }

    /// 执行绑定主体、范围、版本、过滤和策略版本的游标搜索。
    /// 参数：scope 为范围断言，deadline 为原请求期限，arguments 为搜索及游标字段。返回：搜索页、继续位置与截断信息或错误。
    pub(crate) fn search_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = self.require_revision_until(scope, arguments, deadline)?;
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
        let authorizer = self.authorizer_until(deadline)?;
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
        self.engine.with_authorized_revision_reader_until(
            &revision, self.context.principal(), &authorizer, deadline, |reader, snapshot, _deadline| {
        let after = cursor
            .as_ref()
            .map(|cursor| (cursor.last_name.as_str(), cursor.last_id));
        let (items, more) = reader.search_page(snapshot, pattern, after, offset, limit)?;
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
        )            },
        )
    }

    /// 按查询响应编码成本限制页面，超大单节点明确拒绝，避免无进展游标。
    /// 参数：items 为候选节点，base_bytes 为已占用响应字节。返回：准入节点和可选截断原因或错误。
    pub(crate) fn bound_nodes(
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

    /// 读取指定版本节点及覆盖率。
    /// 参数：scope 为范围断言，deadline 为原请求期限，arguments 为 revision/node 字段。返回：节点结果或错误。
    pub(crate) fn node_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision_until(scope, arguments, deadline)?;
        let authorizer = self.authorizer_until(deadline)?;
        self.engine.with_authorized_revision_reader_until(
            &revision,
            self.context.principal(),
            &authorizer,
            deadline,
            |reader, snapshot, _deadline| {
                let node_id = arguments
                    .get("node_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let root = reader
                    .node(snapshot, node_id)?
                    .ok_or(BusinessError::NotFound)?;
                Ok(json!({ "node": root, "coverage": reader.snapshot(snapshot)?.coverage }))
            },
        )
    }

    /// 将相同查询节点按观察大小排列为文本 treemap 行，采集分类作为备注。
    /// 参数：items 为节点迭代器。返回：按大小及名称排序的渲染行。
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

    /// 判断现有工具是否选择文本 treemap，不新增另一套工具。
    /// 参数：arguments 为格式字段。返回：请求 treemap 时为 true。
    fn wants_treemap(arguments: &Value) -> bool {
        arguments
            .get("format")
            .and_then(Value::as_str)
            .is_some_and(|format| format == "treemap")
    }

    /// 读取终端宽度，沿用默认 88 列。
    /// 参数：arguments 为宽度字段。返回：渲染宽度。
    fn treemap_width(arguments: &Value) -> usize {
        arguments.get("width").and_then(Value::as_u64).unwrap_or(88) as usize
    }

    /// 查询实际授权版本的子节点，保留 keyset、未知尺寸及编码门禁。
    /// 参数：scope 为范围断言，deadline 为原请求期限，arguments 为父节点、过滤和游标字段。返回：有界子节点页面或错误。
    pub(crate) fn children_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = self.require_revision_until(scope, arguments, deadline)?;
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
        let authorizer = self.authorizer_until(deadline)?;
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
        self.engine.with_authorized_revision_reader_until(
            &revision, self.context.principal(), &authorizer,
            deadline, |reader, snapshot, _deadline| {
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

    /// 查询大节点并按现有 format 选择 JSON 或 treemap。
    /// 参数：scope 为范围断言，deadline 为原请求期限，arguments 为版本、节点和格式。返回：查询结果或错误。
    pub(crate) fn top_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision_until(scope, arguments, deadline)?;
        let authorizer = self.authorizer_until(deadline)?;
        self.engine.with_authorized_revision_reader_until(
            &revision,
            self.context.principal(),
            &authorizer,
            deadline,
            |reader, snapshot, _deadline| {
                let parent_id = arguments
                    .get("parent_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                reader
                    .node(snapshot, parent_id)?
                    .ok_or(BusinessError::NotFound)?;
                let limit = limit.min(100);
                // 保留旧 revision_layer 的一条前瞻验证，读取量仍随页面而非整棵树增长。
                let mut items = reader.children(snapshot, parent_id, 0, (limit as u64) + 1)?;
                items.truncate(limit);
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
            },
        )
    }
}
