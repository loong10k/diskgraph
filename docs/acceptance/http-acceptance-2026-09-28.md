# Streamable HTTP acceptance (P4-5.1 / P4-5.2)

- transport: Streamable HTTP on loopback
- client: a real HTTP client (not the server's own code path)
- server: the `diskgraph-mcp` binary, started as a separate process

| check | status |
| --- | --- |
| health_reports_no_indexed_data | pass |
| initialize_over_http | pass |
| tool_discovery_matches_the_catalog | pass |
| server_answers_from_its_own_index | pass |
| client_path_is_never_walked | pass |
| client_side_name_is_not_substituted | pass |
| server_response_mentions_only_server_paths | pass |
| batch_is_refused | pass |
| unsupported_version_is_reported | pass |
| write_tool_still_refused_over_http | pass |

passed 10 of 10
