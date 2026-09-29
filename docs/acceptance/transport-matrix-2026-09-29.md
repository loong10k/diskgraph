# Transport matrix record (P4 tasks 5.11 / 5.12, specs MCP-01 / MCP-05 / AI-04)

> Generated 2026-09-29 on a macOS arm64 development host. Hosts: Codex CLI
> (rmcp-based) and Claude Desktop, both real installations. Clients: the hosts
> themselves, plus spec-conformant raw-socket clients for HTTP and legacy SSE.

## Matrix: host x transport

| host | stdio | streamable-http | legacy-sse |
| --- | --- | --- | --- |
| Codex CLI | pass (`codex mcp get` enabled; model-driven `diskgraph_top` completed) | pass (`codex mcp get` reports streamable_http; model-driven call completed, result returned to the model) | not supported by host |
| Claude Desktop | pass (initialize handshake over the registered command) | not supported by host | not supported by host |
| spec client (raw socket) | n/a (stdio is the native form) | pass (10/10 in `accept-http.sh`; notification 202 contract; stream-lifetime regression) | pass (5/5 in `accept-legacy.sh`) |

Per-transport protocol acceptance: `accept-http.sh` (10/10) and
`accept-legacy.sh` (5/5) cover start/status/restart, scope isolation,
authorization, origin policy, quotas, and the notification 202 contract.

## Gap G1: resolved

**Root cause (ours).** The `GET /mcp` server-to-client stream was closed on the
first idle probe: the hold loop treated the socket read timeout as a client
disconnect, so the stream died every few seconds and rmcp-based hosts entered a
reconnect loop. Fixed by treating `WouldBlock`/`TimedOut` as an idle tick that
emits a keep-alive, and pinned by
`http::tests::the_server_stream_survives_read_timeouts`.

**Residual client noise (not a transport failure).** Codex still logs

```
ERROR rmcp::transport::worker: worker quit with fatal:
  Unexpected content type: Some("missing-content-type; body: "),
  when send initialized notification
```

three to five times per session. Evidence that it is client-side and
non-blocking: the same session then completes
`mcp: diskgraph/diskgraph_top (completed)` and the model reports the real item
count, and the message is unchanged across every response variant the server
can legally send (no content type, `application/json`, `text/event-stream`).
The server answers that notification with the spec-conformant empty `202`.
Closing the warning entirely needs a session with the exact rmcp build Codex
ships; it does not affect data flow or authorization.

## 5.12 remote threat-model review

The remote additions were reviewed against `docs/threat-model.md`:

- Remote authentication (A19, bearer + issuer/audience/expiry/signature) —
  implemented and negative-tested; proxy headers are never trusted.
- Origin policy — implemented; hostile origins are refused 403 before auth.
- Quotas — body/response/connection/rate/per-principal implemented; the
  slow-connection hold cannot starve other clients (thread-per-connection with
  read timeouts, covered by the stream-lifetime regression).
- Session/job separation (5.8) — durable job ids survive disconnects;
  a disconnect is never a cancellation (tested on both transports).
- The G1 residual warning does not weaken the model: it occurs on a
  fire-and-forget notification before any tool runs, and no unauthenticated or
  cross-origin request gains data.
- Re-audit trigger: if the rmcp pairing warning is ever eliminated, re-run the
  same negative tests before closing the item.
