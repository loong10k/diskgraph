use super::{
    BrokerQuery, broker_query_request::BrokerQueryRequest, mcp_session_request::McpSessionRequest,
};
use std::time::{Duration, Instant};

#[test]
fn startup_frame_keeps_original_clock_and_rejects_rpc_tail_before_birth() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let encoded = McpSessionRequest::encode(deadline).unwrap();
    for count in McpSessionRequest::PREFIX.len()..encoded.len() {
        assert!(
            McpSessionRequest::decode(&encoded[..count], deadline)
                .unwrap()
                .is_none()
        );
    }
    let (query, adopted) = McpSessionRequest::decode(&encoded, deadline)
        .unwrap()
        .unwrap();
    assert!(query.is_mcp() && query.requires_customer_executor());
    assert!(adopted <= deadline);
    let mut tailed = encoded;
    tailed.extend_from_slice(b"{\"jsonrpc\":\"2.0\"}\n");
    assert!(McpSessionRequest::decode(&tailed, deadline).is_err());
    for length in [0u32, 4096, u32::MAX] {
        let mut invalid = McpSessionRequest::PREFIX.to_vec();
        invalid.extend_from_slice(&length.to_be_bytes());
        assert!(McpSessionRequest::decode(&invalid, deadline).is_err());
    }
    assert!(McpSessionRequest::decode(&vec![b'x'; 4097], deadline).is_err());
    let body = BrokerQueryRequest::encode(
        &BrokerQuery::new("doctor", None, vec![], vec![]).unwrap(),
        deadline,
    )
    .unwrap();
    let mut invalid = McpSessionRequest::PREFIX.to_vec();
    invalid.extend_from_slice(&(body.len() as u32).to_be_bytes());
    invalid.extend_from_slice(&body);
    assert!(McpSessionRequest::decode(&invalid, deadline).is_err());
}
