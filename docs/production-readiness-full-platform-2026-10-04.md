# Full-platform acceptance continuation — 2026-10-04

Continuation of the [full-platform record](production-readiness-full-platform-2026-10-02.md). The complete platform goal remains open.

## Complete Windows attributes and post-lock staging checks (D31 prerequisite, 2026-10-04)

Schema 12 stores a separate fixed 80-byte record for full 128-bit file IDs, u64
volume serials, EOF and native creation/write/change times. A held root chain
constrains relative attribute opens; each native query checks the original
clock/cancellation. Durable grants/fencing refresh within 20ms and are forced
before staging/publication. Native I/O holds no database write lock, leaf
reparse objects are inspected without following, and old content policy stays.
Legacy rows remain uncaptured; conflicting facts produce gaps. Allocated/dedup,
directory and insufficient old identity observations remain Unverified.

Independent review reproduced two committed staging rows after an expired graph
lock wait, hidden by later cleanup. Staging/publication fence callbacks now
check clock/local cancellation after lock waits. The regression retains separate
write events: actual RED 0/1, GREEN 1/0 with expiry and cancellation branches.
[The receipt](benchmarks/windows_observation_acceptance_2026_10_04.json) contains
Core 9/9, Store observation-filter 10/10, Engine observation-filter 24/24 (including
existing cases), final workspace **1089/0/18 across 45 suites**, strict
Clippy/fmt/OpenSpec/release, 14 vendor digests, real stdio 18/18, HTTP/SSE 13/13
and executed UniFFI 19/19. Non-author code APPROVE/architecture CLEAR cover the
44-file final manifest. Ten native Windows and one Engine pipeline regression
require same-source native CI; local macOS does not provide their execution proof.

Release observation point reads executed 18 VM instructions with both 20k/200k
unselected records. Isolated macOS file loads each passed four checks: scans
**0.437/4.391 seconds**, top/children p50/p95 **6.469/8.564 ms** and **6.773/7.999 ms**,
database+WAL **43,442,176/436,916,224 bytes**, direct CLI child maximum RSS
observations **44,826,624/263,307,264 bytes**. Explicit gaps add about 2.1% storage
against D30. Separate observations do not establish paired speedups, a hard RSS
bound or Windows attribute-scan scale cost.

The pinned walk remains path-based; held roots constrain supplementary sampling.
Actual ReFS high IDs, real-provider no-download, old scope encoding provenance,
identity-sensitive history use, native writes, authorized collectors, host/mobile
and signing/deployment gates remain open. No parent checkbox is completed.


## Root namespace rebinding and FFI source boundaries (D32, 2026-10-04)

D31 native CI at `45d732c9` completed 20/22: both Windows Rust jobs showed that an attributes-only handle does not necessarily prevent directory rename. Test-first commit `5963dd0a` also completed 20/22: the renamed and rebound registered root incorrectly validated as `Ok(())`. Both failures and native logs are retained in the D31 receipt. The subsequent fix checks the current drive and each original name relative to retained parent handles against full volume, 128-bit ID, creation and safe directory state. Directory modification-time changes remain allowed. Validation is cooperative and finite, with a post-check race window; each root validation adds native work proportional to root depth. New native CI and a Windows scale measurement are still required.

FFI implementation files now separate API, realm, scanning, JobHandle, shared JobState and aliases. Two fixed real includes preserve UniFFI's old lexical root. Ordinary module extraction initially changed all 19 binding checksums; the corrected build actually restores 19/19 without checksum overrides or wrappers. Chinese contracts use exact Rustdoc-only `cfg_attr(doc, doc = literal)` supplements. The production AST gate parses the real includes and rejects includes elsewhere; two newly reproduced bypasses went RED then GREEN, with 8/8 gate tests. The final local workspace passed 1097/0/18 in 46 suites; Clippy, scoped fmt and OpenSpec strict passed. Same-source native acceptance remains pending; no full-platform parent task is closed.

Release Swift and Kotlin bindings were regenerated. The actual Swift host compiled and ran session, paging, polling, v1 and concurrent system-SQLite calls; four generated Rustdoc pages retain Chinese parameter/return contracts. The [D32 receipt](benchmarks/ffi_structure_root_binding_acceptance_2026_10_04.json) preserves raw RED/GREEN logs, executed 19/19 checksums and the final source manifest. Kotlin native execution and Windows behavior await the new same-source CI.

The first D32 native run at `ca5292b` failed both Windows Rust builds before root regressions: a test-only `path_digest` reexport was unused under `-D warnings`. The followup aligns that import with its sole Unix-test consumer; it keeps strict warnings and production behavior unchanged. All affected FFI targets pass 54/0, FFI Clippy and full-target workspace build pass. The 1097/0/18 result above belongs to the prior full run; new Windows native execution remains required.

The same `ca5292b` Windows release package did pass its 20k-file native drill: scan **15.738 s**, query p50/p95 **27.290/42.900 ms**, DB+WAL **54,067,200 bytes**, four checks passed. This observes the root fix in a real scan, but does not execute the blocked root unit regressions or establish a paired performance improvement. 200k Windows and peak RSS remain unmeasured.


Same-source `c8ff78174e7d7b3f23e8410afc3d16595a48188c` subsequently completed [all 22 CI jobs](https://github.com/loong10k/diskgraph/actions/runs/37155851104). Both Windows Rust versions actually passed all 13 selected distinct native observation/root/publication cases, including renamed root and ancestor bindings. The receipt archives terminal job state and Windows stable/MSRV, Linux ARM and macOS Intel logs. Swift/GRDB and Kotlin native host jobs also passed. This closes D32 native acceptance; namespace races, Windows 200k/RSS, provider, native-write, GUI/mobile/device/signing and production-environment gates remain open.


## Historical size eligibility and source organization (D33)

Actual baseline public APIs returned numeric growth for unknown/read-error nodes and type replacement, treated unknown directories as Same, and exposed placeholder sizes. Corrected isolated regressions at `c8ff781` recorded Core **3/8**, Engine **3/6**, FFI **2/3** (passed/failed); the initial Engine root-directory fixture errors are retained separately. Core now owns pure observation/type eligibility; Engine preserves each side's nullable size and counts only valid Size/Contents changes. Zero and negative known growth remain valid.

Query and comparison now use 17 real object files with their existing public exports and serialization. Independent review confirms 43 of 46 old function bodies unchanged; only growth, changes and compare_entry changed. A three-case AST gate checks these source subtrees, including rejection of direct production path overrides. It is not a crate-wide or macro-expansion certification.

Final local workspace: **1125 passed / 0 failed / 18 ignored in 48 suites**. New regressions: **11/9/5**, structure gate **3/3**, vendor **124/0/2** and **14 unchanged source digests**. Strict Clippy, scoped fmt, explicit include fmt, build and release passed; actual UniFFI **19/19**, stdio **18/18**, HTTP/SSE **13/13** passed. A real file-to-directory replacement also passed **3/3** CLI/MCP checks with equal changes data. Independent code APPROVE and architecture CLEAR matched the 32-file manifest.

| macOS release fixture | Scan seconds | Query p50/p95 ms | DB + WAL bytes | Maximum individual CLI child RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| 20k files | 0.673 | 11.970 / 18.079 | 43,442,176 | 44,990,464 |
| 200k files | 4.371 | 8.536 / 10.459 | 436,916,224 | 263,421,952 |

Each fixture passed 4/4 checks with 32 queries and four clients. Timing includes CLI startup; RSS is measured using macOS time on each actual CLI child, not combined concurrency. These unpaired observations do not establish a speedup, hard RSS bound or history-query throughput. Windows 200k/RSS remain unmeasured. Raw failures, successful commands, protocol outputs, measurement scripts and hashes are in the [D33 receipt](benchmarks/historical_size_acceptance_2026_10_04.json).

Same-source `ca7813cba28d2abdef8309d2d3176893559d68c5` completed [22/22 CI jobs](https://github.com/loong10k/diskgraph/actions/runs/37158622289). Both Windows Rust versions, Linux ARM and macOS Intel logs each execute all 28 selected D33 regressions (Core11/Engine9/FFI5/structure3); the receipt verifies all 32 reviewed source hashes against that exact commit. This accepts the D33 increment. Q04 task3.9 stays open for actual scope compatibility (D34); historical Windows identity and content-version binding remain separate open requirements. No full-platform parent is closed and dangerous CLI/MCP write tools remain disabled.

## Actual historical namespaces (D34)

Both sides being authorized does not establish a shared namespace. Legal owned v1 records can have distinct lossless roots and identical display strings. Corrected regressions recorded Engine **7/2**, expanded isolated baseline **9/2**, and FFI **5/1** (passed/failed). The original APFS directory failure and incorrect FFI scalar assertion are explicitly excluded from defect proof.

Engine and legacy FFI growth now use persisted actual owner/server/scope on their existing readers. Different scopes yield null growth or the existing `different_root` changes tag plus `scope_changed: true`. The scope registration API keeps the root immutable. This trusted helper grants no authority; original response budgets and terminal checks remain, including Engine checks after encoding. Generic authorized cross-root comparison and known same-scope old history remain available. No schema, dependency, owner, thread or export changes were introduced.

Final-source local workspace: **1142 passed / 0 failed / 18 ignored in 49 suites**. Engine namespace **11/11**, FFI new cases **6/6** and all affected growth cases **20/20**; strict Clippy, scoped fmt, include fmt, OpenSpec, build and release passed. Actual UniFFI **19/19**, stdio **18/18**, HTTP/SSE **13/13** passed. Independent code APPROVE and architecture CLEAR matched the final 20-file manifest. See the [D34 receipt](benchmarks/historical_namespace_acceptance_2026_10_04.json) for source hashes, failed observations and raw logs.

Unix offline fixtures use natural invalid-byte display collisions; Windows-enabled fixtures use natural unpaired UTF-16 collisions. Both are synthetic legal metadata records registered through public APIs, with no invalid-directory creation or real migration claim. Linux additionally has two real raw-directory cases; those did not execute on macOS. Existing socket acceptance ran, with no new socket display-collision fixture claimed. Same-source native CI remains required, so Q04 task3.9 and all full-platform parents stay open. Namespace equality does not prove historical file identity or content-version equality.
