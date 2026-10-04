# DiskGraph security and performance hardening — 2026-10-01

[English](security-performance-hardening-2026-10-01.md) · [简体中文](security-performance-hardening-2026-10-01.zh-CN.md)

This implements the approved hardening plan within [implement-diskgraph-platform](../openspec/changes/implement-diskgraph-platform/). It changes source and isolated fixtures only. No branch creation/switch, commit, push, release, real database migration, or installed-host change was performed. Existing README, architecture and locator edits were preserved. The vendored scanner revision, source and digests remain unchanged.

## Implemented behavior

| Area | Behavior and acceptance evidence |
| --- | --- |
| Identity | Immutable request context; issuer/subject-derived principal; token capabilities intersect live database grants. Remote initialization grants no local authority and cannot inherit stdio privileges from an existing database. Binary HTTP/SSE require authentication even on loopback. Origin precedes dispatch for modern and legacy paths. |
| Connections | Legacy sessions bind the authenticated principal. Modern and legacy SSE share a four-connection limit per principal, check expiry/revocation every second, and close expired/revoked streams. Socket tests exercise two principals, token/grant intersection, expiry and revocation. |
| Ownership | Graph schema 5/6 persists revision server/scope and normalized Unicode search. Explicit client scope cannot replace actual ownership. Legacy roots backfill only on a unique scope match; ambiguity remains unbound and denied. CLI/MCP/legacy FFI authorize before reads. |
| Content | Reads charge every byte against the remaining budget and allocate bounded chunks (at most 64 KiB). Partial hashes return an empty digest and an unconfirmed status. Unix component traversal uses root/parent handles and no-follow opens; opened identity, size, mtime/ctime nanoseconds and live authorization are checked during reading. Unsupported platforms refuse. |
| File operations | macOS/Linux primitives bind parent and file handles and use atomic no-replace publication. Copies create exclusive staging, verify both content digests and source stability, and retain the verified staging/source handles until publication/removal. macOS additionally verifies mode, timestamps, flags, ACLs and xattrs. Tampering, parent replacement and target races have library fixture tests. |
| Queries | MCP node/top/children/explore/search use narrow readers. Unicode lowercase substring semantics remain in Rust-normalized search fields with parameterized SQL. Search keyset cursors bind principal, scope, revision, filter, actual ordering and policy version; old cursors are rejected. Explicit offset remains compatible. |
| Bounded output | Trees read by layer; history merges ordered SQL iterators. Node, depth, byte and time limits report truncation. SQLite interruption returns partial history with `complete=false`, `summary_is_partial=true`; unfinished total counts have `node_counts_complete=false`. MCP small-page responses also enforce the byte cap. |
| Concurrency | Request state is separate from a shared Engine; no global service execution mutex. Graph readers use independent connections, bounded caches and SQLite progress deadlines/cancellation; graph writes remain serialized. SQL sorting/counting may inspect more index entries than the returned page. |
| Scan/publication | Existing upstream progress/cancel checked every 20 ms for limits, revocation and cancellation. Iterative conversion avoids recursive traversal and unnecessary node clones. Batches charge encoded JSON and normalized search bytes, not observed file capacity. Owned publication uses SQL from staging instead of re-encoding a full graph. |
| Admission/leases | Capacity checks before enqueue, batches and publication combine data-directory occupancy and measured volume free space (64 MiB reserve). Job quotas/merging use actual principal and a single immediate transaction, which rechecks live grants. Strict conditional claims refuse unexpired jobs even for the same owner name; each generation has an independent cancellation flag. Claims increment fencing, with a 30-second lease and 5-second renewal through conversion/staging/collectors. Expired ownership rescans in a new staging namespace; stale owners cannot write, publish, or finish. |
| Retention/migration | Preview-only `snapshots prune --scope S --keep-last N`; `--apply` deletes old logical history. Protect latest, pins and operation/recovery references, conservatively retaining a whole scope when old references lack revision identity. SQLite consistent migration backups include committed WAL. Graph WAL/NORMAL and control FULL retain their distinct durability requirements. |

Trusted raw store APIs and legacy local API signatures remain compatibility interfaces, deprecated in the architectural contract. Remote callers must use the authorized service. The explicitly trusted in-process HTTP helper retains local compatibility; the server binary and `open_remote` require remote authentication. The systemd example now requires operator-provided authentication configuration.

## Test evidence

Regression probes first failed for unscoped tokens, malicious Origin, foreign revision ownership, revoked publication, hash byte overshoot, capacity admission, in-place modification, WAL backup, narrow reads, leases/fencing, principal job merging, operation planning, and deadline interruption. Implementations were then changed until the corresponding probes passed. New coverage is kept in:

- [Engine hardening fixtures](../crates/diskgraph-engine/tests/hardening.rs): budget, scope isolation, revocation, capacity, large sparse-file metadata charging, tree limits, bounded history, ambiguous legacy ownership and live content withdrawal.
- [MCP hardening fixtures](../crates/diskgraph-mcp/tests/hardening.rs): sockets, request intersection, legacy binding, issuer isolation, expiry, remote/local database isolation and unrelated-corrupt-row narrow reads.
- [Job claim fixtures](../crates/diskgraph-store/tests/job_claim.rs): two real subprocess claimers, expiry, reused owner/new generation, same-owner live-lease refusal, generation-local cancellation, stale completion, quotas/admission and revoked fenced writes.
- [WAL migration fixture](../crates/diskgraph-store/tests/migration_backup.rs), [CLI flow](../crates/diskgraph-cli/tests/cli_flow.rs), FFI unit tests and [operation fixtures](../crates/diskgraph-ops/src/tests.rs): prune protections, compatibility authorization, target/parent races, staged tampering, native ACL/xattr fidelity and retained-source removal.

Final gate results and test counts are recorded after execution below. Only temporary isolated databases/files are used; existing ignored real-project and host tests are not promoted to passing evidence.

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
# fmt: explicit workspace packages, excluding the vendored path dependency
openspec validate implement-diskgraph-platform --strict
DISKGRAPH_BENCHMARK_OUTPUT=measurements.json cargo test --release --offline \
  -p diskgraph-engine --test hardening_benchmark -- --ignored --nocapture
```

## Release measurements

[Raw before/after JSON](benchmarks/hardening-2026-10-01.json) records the platform, baseline revision and every timing distribution. Before is actual `e4d6074` source unpacked into an isolated temporary archive, without a checkout/branch change. Its harness uses original complete-revision loading and tree rendering; after uses narrow/bounded APIs. The source archive's fixture-only dev dependency/harness changes do not modify the production baseline. The same fixture construction creates 32-byte files: wide 20k/200k directories and a 300-level deep directory. Each case has its own process; before and after runs are sequential. Warm top20 has 31 samples, tree 11, and each of four concurrent readers 31. Scan has one measurement per case, so differences are observations, not statistical guarantees.

| Fixture | top20 p50 ms before → after | top20 p95 ms before → after | 4-reader worst p95 ms before → after | tree p95 ms before → after |
| --- | --- | --- | --- | --- |
| 20k wide | 11.701 → 0.134 | 12.273 → 0.273 | 46.114 → 1.370 | 26.959 → 5.707 |
| 200k wide | 105.217 → 0.142 | 112.234 → 0.217 | 411.494 → 1.752 | 291.192 → 40.914 |
| 300 deep | 0.380 → 0.129 | 0.490 → 0.221 | 1.590 → 2.185 | 0.491 → 0.485 |

Before tree renders all matching nodes at depth 2; after uses the default 100-node/64 KiB/depth-2 budget and reports truncation. These tree measurements compare the old behavior with the intended bounded behavior, not identical output volumes. The paired full-load control in the current database is also preserved in the JSON.

| Fixture | scan seconds before → after | scan-phase process peak RSS MiB before → after | database MiB before → after | final WAL bytes before / after |
| --- | --- | --- | --- | --- |
| 20k wide | 0.193 → 0.450 | 37.172 → 34.484 | 17.727 → 26.980 | 0 / 0 |
| 200k wide | 1.908 → 3.369 | 269.766 → 213.719 | 176.844 → 272.004 | 0 / 0 |
| 300 deep | 0.037 → 0.042 | 13.344 → 14.062 | 1.051 → 1.742 | 0 / 0 |

For 200k, top20 p95 improves about 516× and scan-phase peak RSS falls about 21%; scan time rises about 77% and physical database size about 54%. Normalized fields/indexes, authorization/fencing and publication validation cost time and storage. This delivery does **not** claim faster scans or lower database capacity. The small deep-directory concurrent p95 regressed in the final run; improvements must not be generalized to every workload. Whole-run RSS also includes the full-load/tree/concurrent controls; it must not be interpreted as narrow-query RSS. WAL zero is an end-of-scan checkpoint sample, not a measurement of peak transient WAL writes. No enforced scanner RSS ceiling is inferred from these observations.

## Compatibility and remaining limits

- Old remote grants require reissue for the new issuer/subject principal; old search cursors require a fresh query. Wire data fields remain, with additive diagnostics. Directory offset pagination remains; new keyset cursors apply to search.
- Old running jobs are initialized from heartbeat + 30 seconds and await expiry before reclaim. Migration failure prevents service opening. Unbound legacy revisions require administrator reindexing; no caller-provided scope can bind them.
- Upstream still constructs a whole tree. Cancellation occurs cooperatively at upstream directory/parallel-work boundaries and may overshoot briefly. Streaming and a strict scan memory cap were excluded by the approved plan.
- Logical prune frees SQLite pages but does not guarantee physical file shrinkage; no automatic VACUUM/history deletion is introduced. Capacity still measures actual files. Existing stale staging and backups therefore remain visible capacity costs.
- Copy verification and native no-replace do not create a global filesystem transaction. Content changes after observation and compensating rollback conflicts remain explicit failure/recovery states. Native fidelity checks reject unsupported conditions.
- CLI/MCP dangerous file tools remain disabled. Linux/Windows native writes, mobile providers, cloud placeholder behavior and installed Swift/Kotlin host/UI end-to-end acceptance were not run. macOS fixture ACL/xattr verification is evidence for this library path only, not permission to enable remote writes. Existing real-volume/recovery tests remain ignored unless their required environments are provided.
- SQLite's progress hook interrupts executing statements; lock wait uses its separate bounded busy timeout. It does not cancel an arbitrary filesystem collector or promise hard wall-clock scheduling. There is no cross-database atomic commit promise. Graph WAL/NORMAL may lose the newest reconstructible committed index after power loss; control FULL preserves the stronger intended durability, subject to the underlying filesystem/hardware guarantees.

OpenSpec remains active and unarchived. Cross-platform tasks stay unchecked; this record does not certify the entire original platform roadmap.

## Full-code-review follow-up

The follow-up work is tracked in OpenSpec tasks 13.1–13.7. Impact and related queries now authorize the revision's persisted owner and page through relations with explicit truncation. HTTP enforces request-line, header-count, header-byte, body, and absolute read deadlines; SSE liveness writes are bounded. HTML report values are escaped in both markup and embedded JSON contexts. CLI `serve` forwards remote authentication and Origin options, and the socket acceptance fixture uses a signed test token and isolated grants.

Operation apply rechecks live scope/action/policy grants, plan expiration, complete SHA-256 plan digest, source fingerprint and total byte budget. Copy, Move and Restore check destination scope; operation claims use an immediate SQLite transaction against real source and destination identity. Old plans lack the new digest/fingerprint and must be planned again. Directory sources and Windows specialist execution currently return `unsupported` where the verified budget or process-tree deadline cannot be guaranteed. The Unix specialist deadline covers descendants that retain output pipes.

Cancellation is persisted across Engine instances and is checked at the publication fence. Reclaimed jobs use a fresh fencing namespace and clear only stale staging belonging to that job. FFI control databases are derived from the lossless graph path; a legacy shared control database is reused only when unique ownership can be proved, otherwise reopening requires administrator reindexing. The TUI loads wide directories in pages of 512; name sorting is page-local. A sparse index covers unknown-size child counts and an ordered-path index prevents a temporary sort for history iteration.

The prior performance table was measured **before** this follow-up. An additional isolated release run of the current tree gave the following results; arrows compare the earlier hardened run with this follow-up run, not an A/B run under identical concurrent host load. See [raw measurements](benchmarks/review-followup-2026-10-01.json).

| Fixture | Scan s | Scan peak RSS MiB | DB MiB | Top20 p95 ms | Bounded tree p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 20k wide | 0.450 → 0.361 | 34.5 → 34.9 | 27.0 → 29.9 | 0.273 → 0.244 | 5.707 → 1.861 |
| 200k wide | 3.369 → 4.506 | 213.7 → 213.6 | 272.0 → 301.0 | 0.217 → 0.230 | 40.914 → 10.545 |
| 300 deep | 0.042 → 0.041 | 14.1 → 14.0 | 1.7 → 2.0 | 0.221 → 0.150 | 0.485 → 0.322 |

The 200k scan and physical database are slower/larger in this run, while bounded tree time improved. Query p95 differences at sub-millisecond scale may be noise; neither run proves a universal speedup. Relation traversal opens a narrow read connection per page/direction, so high-degree impact queries may incur connection overhead. Candidate selection with a positive target still loads the complete revision to preserve global ranking and ancestor evidence; a store-side eligible-directory index is future work. Directory pages remain offset-based and deep offsets still traverse earlier rows. No strict RSS or wall-clock bound for the upstream scanner is claimed.


## Final executed gates

2026-10-01, local macOS:

| Gate | Observed result |
| --- | --- |
| `cargo test --workspace --locked` | 39 suites; 470 passed, 0 failed, 12 ignored |
| Vendored scanner `cargo test --offline --quiet` | 124 passed, 0 failed, 2 ignored |
| Explicit release fixture | One aggregate benchmark passed for each of baseline/current; three isolated child scenarios |
| Explicit workspace package fmt check | Passed; vendor excluded |
| Workspace/all-targets/locked Clippy, `-D warnings` | Passed |
| OpenSpec strict | `implement-diskgraph-platform` valid |
| Opening vendor SHA-256 baseline | All 20 non-target source/manifest/pin/digest files unchanged |
| Git | Original main branch and `e4d6074` HEAD unchanged; changes uncommitted, no push/release |

Tasks 12.1–12.7 are checked within this local acceptance scope. Task 12.8 and existing unfinished platform/host tasks stay unchecked. Subsequent acceptance requires the corresponding Linux/Windows/device environments; no automatic dangerous-tool enablement, installation, or real-data migration is authorized by this record.

## Review follow-up validation (2026-10-02)

| Gate | Observed result |
| --- | --- |
| `cargo test --workspace --locked --offline --quiet` | 39 suites; 518 passed, 0 failed, 12 ignored |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| Non-vendor workspace `cargo fmt --check`; `git diff --check` | Passed |
| `openspec validate implement-diskgraph-platform --strict` | Valid |
| `actionlint .github/workflows/release.yml` | Passed; registry jobs wait for binary matrix |
| Real loopback HTTP acceptance with signed isolated token | 11/11 passed |
| CLI `serve` child startup with remote auth | Separate listener started |
| Current release benchmark | 20k/200k wide and 300-deep fixtures passed; measurements above |
| Vendored upstream source/pin/digests | Unchanged from HEAD; dedicated pin/digest suite 4/4 passed |

At the original 2026-10-01 acceptance point, tasks 13.1–13.5 and 13.7 were verified locally; 13.6 still required positive-target candidate preparation and request-scoped relation-reader reuse. Linux/Windows native writes and installed mobile hosts were unverified, and that original run made no commit, push or publication. Later increments below supersede its remaining-work status.

## D34 native follow-up, 2026-10-04

Actual historical namespace eligibility passed [22/22 native CI jobs](https://github.com/loong10k/diskgraph/actions/runs/37161135994) at `407125f62fda994826a7858737b22fa95efe4cb4`. All 20 reviewed source hashes match that commit. Both Windows Rust versions and macOS Intel executed Engine11/FFI6; Linux ARM also executed two real raw-byte filesystem cases. The [D34 receipt](benchmarks/historical_namespace_acceptance_2026_10_04.json) preserves exact cases, raw logs and earlier failed observations. Existing authorization, bounded responses and terminal checks remain; dangerous CLI/MCP write tools remain disabled.

At D34, Q-04 task3.9 still required the complete settings, size-basis and provider compatibility matrix. D35 subsequently accepted this matrix and canonical coverage semantics at `2450ab1`, with [22/22 same-source native jobs](https://github.com/loong10k/diskgraph/actions/runs/37164196395) and 26 actual selected cases in each of four raw logs; see the [D35 receipt](benchmarks/native_input_history_matrix_acceptance_2026_10_04.json). Git library configuration/filter isolation, unborn/failure semantics and shared probe budgets have native acceptance under 15.13b/c/d and D29; task8.7/15.13 still requires an authorized sampling entry, persistence and actual revision publication acceptance. No full-platform readiness parent is complete.

## D36 local follow-up, 2026-10-04

Scoped Git capture, narrow TUI preparation with original deadlines and terminal authorization, and shared raw candidate-header admission are implemented. Final-source workspace passed 1233/0/18 across 51 suites; seven quality gates, release stdio18/18, HTTP/SSE13/13 and UniFFI checksums19/19 passed. Both real workspace failures and subsequent strict-Clippy failures remain archived. The [Git capture](benchmarks/scoped_git_capture_acceptance_2026_10_04.json) and [TUI/query](benchmarks/tui_budget_acceptance_2026_10_04.json) receipts separate local phases from pending same-source native execution. Task13.6 awaits that native gate; task8.7/15.13, real providers, native writes, host/mobile/device/signing and production gates remain open.
