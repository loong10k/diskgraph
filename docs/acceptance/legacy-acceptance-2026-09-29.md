# Legacy HTTP+SSE acceptance (P4-5.6 / P4-5.7)

- protocol: MCP HTTP+SSE, 2024-11-05 wire contract
- client: spec-conformant legacy client (raw socket, no SDK)
- server: the `diskgraph-mcp` binary, separate process, legacy opted in

| check | status |
| --- | --- |
| endpoint_event_contract | pass |
| legacy_post_acknowledged_202 | pass |
| response_arrives_as_message_event | pass |
| unknown_session_refused_404 | pass |
| default_off_reports_disabled | pass |

passed 5 of 5

A real old MCP host remains a deployer-matrix item (task 5.11);
this acceptance proves the adapter speaks the old protocol exactly.
