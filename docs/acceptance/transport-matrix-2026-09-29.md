# Transport matrix record (P4 tasks 5.11 / 5.12, specs MCP-01 / MCP-05 / AI-04)

> Generated 2026-09-29 on a macOS arm64 development host. Hosts: Codex CLI
> (rmcp-based) and Claude Desktop, both real installations. Clients: the hosts
> themselves, plus spec-conformant raw-socket clients for HTTP and legacy SSE.

## Matrix: host x transport

| host | stdio | streamable-http | legacy-sse |
| --- | --- | --- | --- |
| Codex CLI | pass (`codex mcp get` enabled; model-driven `diskgraph_top` completed) | **fail — see gap G1** (`codex mcp get` enabled over `url`; rmcp worker rejects the notification response) | not supported by host |
| Claude Desktop | pass (initialize handshake over the registered command) | not supported by host | not supported by host |
| spec client (raw socket) | n/a (stdio is the native form) | pass (10/10 in `accept-http.sh`; plus 202-notification contract) | pass (5/5 in `accept-legacy.sh`) |

Per-transport protocol acceptance: `accept-http.sh` (10/10) and
`accept-legacy.sh` (5/5) cover start/status/restart, scope isolation,
authorization, origin policy, quotas, and the notification 202 contract.

## Gap G1: rmcp worker rejects the notification response (recorded, not fixed)

Symptom: rmcp-based hosts (Codex) quit the transport with
`Unexpected content type: Some("missing-content-type; body: ")` right after
POSTing `notifications/initialized`.

Server side evidence (curl against the same binary): the notification POST is
answered exactly per the Streamable HTTP contract — `202 Accepted`,
`Content-Type: application/json`, `Content-Length: 0`. Variants tried:
no Content-Type, application/json. GET /mcp was also exercised as a held
event-stream (silent hold, keep-alive comments). Every variant fails at the
same point, while curl and the raw-socket client accept the same responses.

Assessment: the mismatch is in the **client** (rmcp 3.4 worker) side of the
POST-notification/GET-stream pairing, not in this server's contract. Closing
it needs a debugging session with the rmcp client build Codex ships (its
expectations for the notification response and for the GET stream's first
event). Recorded as a known gap rather than silently dropped.

Workaround for rmcp-based hosts: the **stdio** transport passes the same
hosts (see `host-acceptance-2026-09-29.md`).

## 5.12 remote threat-model review

The remote additions were reviewed against `docs/threat-model.md`:

- Remote authentication (A19, bearer + issuer/audience/expiry/signature) —
  implemented and negative-tested; proxy headers never trusted.
- Origin policy — implemented; hostile origins refused 403 before auth.
- Quotas — body/response/connection/rate/per-principal implemented; the
  slow-connection hold cannot starve other clients (thread-per-connection
  with read timeouts).
- Session/job separation (5.8) — durable job ids survive disconnects;
  disconnect is never a cancellation (tested on both transports).
- Gap G1 does not weaken the model: the rmcp refusal happens **before** any
  tool executes, so no unauthenticated or cross-origin request gains data.
- Remaining open item for P5+: the rmcp pairing session (G1) must be
  re-audited once fixed, with the same negative tests re-run.
