//! 真实 bearer/Origin/TCP 关系响应编码的连续撤权；合法导入不替代扫描验收。

#[test]
fn encoded_socket_related_remembers_grant_revocation_even_if_restored() {
    crate::history_budget_tests::terminal_withdrawal("diskgraph_related", false, true);
}

#[test]
fn encoded_socket_explain_remembers_grant_revocation_even_if_restored() {
    crate::history_budget_tests::terminal_withdrawal("diskgraph_explain", false, true);
}

#[test]
fn encoded_socket_impact_remembers_grant_revocation_even_if_restored() {
    crate::history_budget_tests::terminal_withdrawal("diskgraph_impact", false, true);
}

#[test]
fn encoded_socket_candidates_remembers_grant_revocation_even_if_restored() {
    crate::history_budget_tests::terminal_withdrawal("diskgraph_candidates", false, true);
}

#[test]
fn encoded_socket_related_expiry_preserves_bounded_partial_diagnostic() {
    crate::history_budget_tests::relation_encoded_expiry("diskgraph_related");
}

#[test]
fn encoded_socket_explain_expiry_preserves_bounded_partial_diagnostic() {
    crate::history_budget_tests::relation_encoded_expiry("diskgraph_explain");
}

#[test]
fn encoded_socket_impact_expiry_preserves_bounded_partial_diagnostic() {
    crate::history_budget_tests::relation_encoded_expiry("diskgraph_impact");
}

#[test]
fn encoded_socket_candidates_expiry_preserves_bounded_partial_diagnostic() {
    crate::history_budget_tests::relation_encoded_expiry("diskgraph_candidates");
}
