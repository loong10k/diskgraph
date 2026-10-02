//! 既有查询诊断及游标上下文。

use diskgraph_core::{CursorContext, Incompatibility};

/// 输出历史不兼容原因的稳定 wire 名称。
/// 参数：reason 为不兼容原因。
/// 返回：既有静态字符串。
/// Renders an incompatibility reason as the stable wire name.
pub fn incompatibility_name(reason: Incompatibility) -> &'static str {
    match reason {
        Incompatibility::DifferentRoot => "different_root",
        Incompatibility::DifferentVolume => "different_volume",
        Incompatibility::UnknownVolume => "unknown_volume",
        Incompatibility::DifferentSettings => "different_settings",
        Incompatibility::OutOfOrder => "out_of_order",
        Incompatibility::IncompleteCoverage => "incomplete_coverage",
    }
}
/// 构造旧兼容查询的游标上下文。
/// 参数：principal_binding、scope_id、revision_id 为查询身份。
/// 返回：借用的兼容上下文，保留排序与策略默认值。
/// Builds the cursor context for one query identity.
pub fn cursor_context<'a>(
    principal_binding: &'a str,
    scope_id: &'a str,
    revision_id: &'a str,
) -> CursorContext<'a> {
    CursorContext {
        principal_binding,
        scope_id,
        revision_id,
        filter_binding: "",
        sort_binding: "size_desc,name_asc",
        policy_version: 0,
    }
}
