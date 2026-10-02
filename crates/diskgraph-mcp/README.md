# diskgraph-mcp

The Model Context Protocol server for
[DiskGraph](https://github.com/loong10k/diskgraph), over three transports:
stdio (local process), Streamable HTTP (authenticated, including loopback), and
the legacy HTTP+SSE adapter.

## Install

Binary channels (recommended):

```bash
brew install loong10k/diskgraph/diskgraph     # includes diskgraph-mcp
npx -y diskgraph-mcp                            # thin installer
```

Or from source:

```toml
[dependencies]
diskgraph-mcp = "0.2"
```

## Running

```bash
diskgraph-mcp --data-dir ~/.diskgraph --profile all
# Streamable HTTP on loopback:
diskgraph-mcp --transport streamable-http --port 8737 --auth-key-file "$ISSUER" "$AUDIENCE" /secure/path/mcp.key
# Legacy SSE (opt-in):
diskgraph-mcp --transport legacy-sse --auth-key-file "$ISSUER" "$AUDIENCE" /secure/path/mcp.key
```

Registering with an MCP host, e.g. Codex:

```bash
codex mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all
```

## Profiles

`read-minimal` (common reads), `read-full` (every metadata query),
`manage` (scope and index management), `all`. A profile advertises only
the tools it serves; catalog families this build does not implement answer
`unsupported` rather than disappearing.

## Guarantees

- All HTTP/SSE binds require authentication, including loopback: HS256 JWTs with
  issuer, audience, and expiry verified before dispatch. Token capabilities
  intersect live database grants; remote startup never bootstraps local admin.
  Provision grants for the issuer/subject-derived principal through a trusted
  local administrator. The systemd unit reads issuer/audience from
  `/etc/diskgraph/mcp.env` and a 32+ byte key from `/etc/diskgraph/mcp.key`.
  On Unix the key must be a regular file inaccessible to group and others;
  on Windows the operator must restrict its ACL to the service identity.
  The legacy `--auth ISSUER AUDIENCE KEY` form remains for compatibility but
  exposes the key in process arguments and must not be used for deployment.
- Body, connection, rate, and per-principal job limits are enforced, and
  a slow consumer cannot stall a scan.
  Rate state is per service instance and client IP, with at most 4096 buckets
  and 64-byte keys. Only buckets idle for 60 seconds are reclaimed; a full
  table rejects new IPs with 429, including authenticated users, and active
  buckets can continue occupying it. `retry_after_ms` is a retry hint, not a
  guarantee that capacity returns after that interval. NAT clients share a
  bucket, and restart clears this in-memory state.
- Trusted proxies are exact IPs. They must replace untrusted forwarded input
  with the actual source, or append the actual peer to the chain. Duplicate
  `X-Forwarded-For` fields are combined in received order within the header
  budget. The server walks at most 32 valid IPs from right to left through
  configured proxies, stopping at the first untrusted hop. Invalid or excessive
  chains fall back to the direct peer, sharing its quota. IPv6 and mapped IPv4
  spellings are canonicalized; forwarding never establishes a principal.
- Connection identity and business job ids are separate, so a
  disconnected client can reconnect and still query its job.
- Legacy delivery reserves capacity before 202 and tool execution: 64 messages
  and 16 MiB per session, 64 MiB across one listener, including pending tools,
  encoding, queued frames and writes. A reservation starts at the configured
  response limit plus 22 SSE framing bytes and shrinks to actual encoded bytes.
  Admission refuses congestion with 429. A single reservation above 16 MiB, or
  a budget unable to fit a JSON-RPC error with the original request id, gets
  413 before execution. Oversized tool results become a bounded JSON-RPC
  `response_too_large` error. Small HTTP refusal diagnostics have a fixed size
  even when a configured budget is too small to carry any protocol error.
  The default 4 MiB limit admits at most three simultaneous worst-case
  reservations per session and fifteen per listener; smaller encoded frames
  free the difference. Queued-byte budgets do not cover tool-internal `Value`
  memory, allocator overhead, kernel socket buffers or a strict RSS limit.
- Legacy data writes retry partial nonblocking progress under one absolute
  `read_timeout` deadline, including control-lock admission and SQLite reads.
  Each chunk rechecks token expiry and the persistent authorization generation;
  policy, grant or scope changes conservatively close older result streams,
  including unrelated subjects' changes. Job heartbeats do not invalidate them.
  Reconnect after closure and query durable job state; bytes already handed to
  the kernel cannot be retracted. Outer idle liveness checks may wait on policy
  access, so this is not a strict one-second revocation/cleanup SLA. Receiver
  closure frees queued frames; pending tool reservations remain charged until
  their work exits. Admission-after-disconnect races close and log the session.
  The public raw `legacy::SessionRegistry` remains a trusted internal
  compatibility interface and is never used by remote HTTP.
- Stop existing services and hosts before upgrading to control schema 6. The
  upgrade has a SQLite consistent backup and transactional triggers; an older
  program rejects the new schema when reopening. A surviving old connection
  does not acquire these transport fixes automatically. This is not a rolling
  mixed-version security upgrade.
- The server is read-only in every profile this build ships.

## License

MIT
