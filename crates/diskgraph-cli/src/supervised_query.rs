use crate::command::Command;
use crate::scope_action::ScopeAction;
use diskgraph_engine::supervisor_service::BrokerQuery;

/// 参数：已由真实Clap解析的命令；返回：固定只读描述或尚未接入的命令。
/// 不复制原argv，不转交数据目录、主体、环境、FD或扫描配置；资源授权仍进入原Engine。
pub(crate) fn from_command(command: &Command) -> Option<BrokerQuery> {
    let mut options = Vec::new();
    let mut switches = Vec::new();
    let mut subcommand = None;
    macro_rules! values {
        ($($key:literal => $value:expr),* $(,)?) => {
            $(options.push(($key, $value.to_string()));)*
        };
    }
    let name = match command {
        Command::Doctor => "doctor",
        Command::Scope {
            action: ScopeAction::List,
        } => {
            subcommand = Some("list");
            "scope"
        }
        Command::Scope {
            action: ScopeAction::Show { scope },
        } => {
            subcommand = Some("show");
            values!("scope" => scope);
            "scope"
        }
        Command::Status { job } => {
            values!("job" => job);
            "status"
        }
        Command::Snapshots {
            scope,
            limit,
            offset,
            action: None,
        } => {
            if let Some(scope) = scope {
                values!("scope" => scope);
            }
            values!("limit" => limit, "offset" => offset);
            "snapshots"
        }
        Command::Tree {
            scope,
            revision,
            depth,
            html: None,
            anonymize,
            min_bytes,
        } => {
            values!("scope" => scope, "depth" => depth, "min-bytes" => min_bytes);
            if let Some(revision) = revision {
                values!("revision" => revision);
            }
            if *anonymize {
                switches.push("anonymize");
            }
            "tree"
        }
        Command::Node { scope } => {
            values!("scope" => scope);
            "node"
        }
        Command::Children {
            scope,
            parent_id,
            limit,
            offset,
            min_bytes,
            unknown_only,
        } => {
            values!("scope" => scope, "parent-id" => parent_id, "limit" => limit, "offset" => offset);
            if let Some(min_bytes) = min_bytes {
                values!("min-bytes" => min_bytes);
            }
            if *unknown_only {
                switches.push("unknown-only");
            }
            "children"
        }
        Command::Top {
            scope,
            parent_id,
            limit,
        } => {
            values!("scope" => scope, "parent-id" => parent_id, "limit" => limit);
            "top"
        }
        Command::Search {
            scope,
            pattern,
            offset,
            limit,
            cursor,
        } => {
            values!("scope" => scope, "pattern" => pattern, "offset" => offset, "limit" => limit);
            if let Some(cursor) = cursor {
                values!("cursor" => cursor);
            }
            "search"
        }
        Command::Explore {
            scope,
            node_id,
            max_depth,
            max_nodes,
        } => {
            values!("scope" => scope, "node-id" => node_id, "max-depth" => max_depth, "max-nodes" => max_nodes);
            "explore"
        }
        Command::Growth {
            scope,
            before,
            after,
            path,
        } => {
            values!("scope" => scope, "before" => before, "after" => after, "path" => path);
            "growth"
        }
        Command::Changes {
            scope,
            before,
            after,
        } => {
            values!("scope" => scope, "before" => before, "after" => after);
            "changes"
        }
        Command::Explain {
            scope,
            revision,
            entity,
            limit,
            after_edge,
        } => {
            values!("scope" => scope, "revision" => revision, "entity" => entity, "limit" => limit);
            if let Some(after_edge) = after_edge {
                values!("after-edge" => after_edge);
            }
            "explain"
        }
        Command::Related {
            scope,
            revision,
            entity,
            relation,
            outgoing,
            limit,
            after_edge,
        } => {
            values!("scope" => scope, "revision" => revision, "entity" => entity, "limit" => limit);
            if let Some(relation) = relation {
                values!("relation" => relation);
            }
            if let Some(after_edge) = after_edge {
                values!("after-edge" => after_edge);
            }
            if *outgoing {
                switches.push("outgoing");
            }
            "related"
        }
        Command::Impact {
            scope,
            revision,
            entity,
            max_depth,
            max_edges,
        } => {
            values!("scope" => scope, "revision" => revision, "entity" => entity, "max-depth" => max_depth, "max-edges" => max_edges);
            "impact"
        }
        Command::Candidates {
            scope,
            target_bytes,
        } => {
            values!("scope" => scope, "target-bytes" => target_bytes);
            "candidates"
        }
        _ => return None,
    };
    BrokerQuery::new(name, subcommand, options, switches).ok()
}
