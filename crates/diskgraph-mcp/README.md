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
- Connection identity and business job ids are separate, so a
  disconnected client can reconnect and still query its job.
- The server is read-only in every profile this build ships.

## License

MIT
