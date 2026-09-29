# Linux package record (P4 task 5.10, spec ST-05 / AI-01)

> Generated 2026-09-29 on a macOS arm64 development host.

## Delivered

- `scripts/package-linux.sh` — the packaging script, **run on the Linux
  deployment host**: release build (`--locked`), bundle layout
  (`bin/`, `deploy/`, SHA256SUMS, MANIFEST, LICENSE, DEPENDENCIES), and a
  smoke test covering start, index, query, doctor, and a restart query
  against local SQLite.
- `deploy/diskgraph-mcp.service` — hardened systemd unit: unprivileged user,
  `ProtectSystem=strict`, loopback default, `StateDirectory` for the two
  SQLite databases, explicit local-only rule (ST-05).
- Machine-checked contract: `crates/diskgraph-mcp/tests/deploy_contract.rs`
  (5 checks) pins the unit's hardening fields, loopback default, local-only
  rule, and the packaging script's restart-safety smoke.

## Not verified here (honest gap)

A real Linux run (start/stop/restart/upgrade via systemd, local SQLite under
/var/lib/diskgraph) **has not been executed**: this host is macOS. Cross
compilation from macOS was attempted and stopped at the link stage (zig cc:
musl targets are rejected by cc-rs's triple, gnu needs environment-supported
glibc stubs); the chosen design builds **on the Linux host**, which needs no
cross toolchain.

## Deployment-host steps

1. `git clone` + `./scripts/package-linux.sh`
2. Verify the smoke lines (index completed, restart query, doctor healthy)
3. `install` the binaries and unit per the unit's header comments
4. Network-facing: add `--auth ISSUER AUDIENCE KEY` + TLS front
   (the server refuses non-loopback binds without both)
