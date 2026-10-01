# Desktop read-only readiness record — 2026-10-02

Scope: macOS, Linux and Windows CLI plus MCP stdio, Streamable HTTP and legacy SSE. File mutation tools remain disabled. OpenSpec `implement-diskgraph-platform` RE-06 and Q-09 are the acceptance source.

| Gate | macOS arm64 | macOS x86_64 | Linux x86_64 | Windows x86_64 |
| --- | --- | --- | --- | --- |
| Current source tests, fmt and Clippy | Local pass | CI configured; run pending | CI configured; run pending | CI configured; run pending |
| Actual CLI/MCP binary protocol acceptance | Local release binaries: stdio 11/11, HTTP/SSE 13/13 | Native release job pending | CI/release run pending | CI/release run pending |
| v7 to v8 consistent backup migration | Isolated fixture pass | CI run pending | CI run pending | CI run pending |
| Packaged artifact SHA and upgrade/rollback drill | Local binary hashes and old-binary rollback 7/7; package pending | Pending | Pending | Pending |
| Capacity/concurrent operation in target environment | Isolated 20k/200k measurements; deployment drill pending | Pending | Pending | Pending |

Local evidence: macOS arm64, Rust 1.98.1, source HEAD `e4d6074` with uncommitted changes. `cargo test --workspace --all-targets --locked --quiet` passed on 2026-10-02; ignored tests include real-host cases. The release 20k/200k fixture was run explicitly. `cargo clippy --workspace --all-targets --locked -- -D warnings` passed. Vendored scanner digest test passed after restoring its exact pinned bytes. The native release binaries built with `cargo build --release --locked -p diskgraph-cli -p diskgraph-mcp` report version 0.3.0 and passed both portable acceptance scripts. Their current SHA-256 values are:

| Binary | SHA-256 |
| --- | --- |
| `target/release/diskgraph` | `1b84d909d10b66eb1bf009ecfe85096d0c5cb4ac3421026008b98c94240dffb1` |
| `target/release/diskgraph-mcp` | `1e6ddf4896c01472ecc6d762eddfb13fdb5da18e3f3ae74ecdf08fe8459d5a0b` |

These are local binaries from an uncommitted tree, not distributable release digests. The release workflow computes archive digests separately. The CI matrix builds and tests actual binaries on all three OS families, then runs the same isolated CLI/stdio and signed-token HTTP/SSE acceptance. The listener reads its verifier key from a protected file; the shipped systemd unit does not place it in process arguments. Native release jobs repeat the protocol check before packaging, including the dedicated macOS Intel runner; cross-built Linux arm64 still needs target-host execution before an architecture-specific readiness claim.

The [raw release benchmark](benchmarks/readiness-2026-10-02.json) uses separate isolated processes, 32-byte files and one explicit rebuildable root fact. The positive-target candidate query p95 was 12.38 ms with a full revision load versus 0.35 ms with narrow read at 20k files, and 106.04 ms versus 0.36 ms at 200k files. The deep 300-level case was 0.54 ms versus 0.48 ms. Narrow sampling used 11 calls and full-load sampling 3 calls; these warm-cache observations are not an SLA. Scan-phase peak RSS was about 37 MB at 20k and 225 MB at 200k; whole-process peak includes full-load controls and is not a narrow-query RSS estimate. Database files were about 31 MB and 316 MB respectively. The scanner still has no strict RSS ceiling.

To repeat locally, build the CLI and MCP binaries, then run `python scripts/accept-readonly-stdio.py` and `python scripts/accept-readonly-http.py`. Set `DISKGRAPH_ACCEPT_BIN_DIR` to a release-binary directory to test exactly those binaries. The scripts create temporary data directories, register a fixture scope, issue a short-lived test token and grant only `metadata:read`; they test denied anonymous, hostile-Origin, ungranted and revoked requests. They never touch a production control database.

Schema 8 creates candidate-size and evidence-relation indexes. Migration uses a SQLite consistent pre-upgrade backup under `migration_backups/`; the v7→v8 test checks preserved committed data and both new indexes. A separate macOS binary drill archived HEAD `e4d6074` without changing branches, built its schema-4 CLI in isolation and ran `python scripts/accept-readonly-upgrade.py --old-cli OLD --new-cli NEW`: 7/7 checks passed, including revision/tree preservation, both database backups, refusal of the upgraded directory by the old binary, and successful reads after restoring both backups. For an operational rollback, stop CLI/MCP and job workers, retain the failed upgraded data directory as evidence, restore both graph and control databases from the same pre-upgrade backup set, then run the previous binary against the restored directory. Do not point an older binary at a newer schema. This procedure still needs an actual packaged-binary drill on each target OS before the gate can close. WAL/NORMAL can lose recent rebuildable graph commits on power loss; control FULL protects its own transactions, and the two databases have no cross-database atomicity.

Current decision: **not yet production ready across all three OS families**. A configured workflow and local macOS success do not replace Linux/Windows runner results, archive verification, rollback or controlled runtime observation.
