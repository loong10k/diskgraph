# DiskGraph Architecture

[English](DiskGraph-Architecture.md) | [简体中文](DiskGraph-Architecture.zh_CN.md)

> **Purpose**: Define the shared engine's boundaries, components, flows and acceptance criteria for maintainers, native developers, agent integrators, testers and operators.
>
> **Document version**: 1.1, independent of software and schema versions\
> **Source baseline**: `a89e57a`; workspace `0.1.0`\
> **Owner**: DiskGraph maintainers; individual reviewers remain to be assigned\
> **Verified**: 2026-09-28; **Status**: pending review\
> **Deployment scope**: existing embedded libraries; planned local tools, single-host servers and restricted mobile hosts. Keep the project private under the agreed release policy.

```text
Authorized directories / platform observations
        |
        v
[DiskGraph: scan -> snapshot -> evidence graph -> bounded query]
        |                                             |
        v                                             v
Rust / FFI / CLI / MCP                         candidates + reasons
                                                      |
                                            plan -> approve -> act
```

**Existing foundation** means source exists and baseline tests passed locally, not that the product is stable. **Target** means planned OpenSpec behavior. **Unverified** means target-environment evidence is missing. Unless explicitly marked current, runtime, authorization and execution mechanisms below describe the target architecture.

The [OpenSpec change](../openspec/changes/implement-diskgraph-platform/proposal.md) owns requirements and acceptance. The [technical design](DiskGraph-Technical-Design.md) owns engineering detail; the [command reference](command-reference.md) owns the interface catalog; [tasks](../openspec/changes/implement-diskgraph-platform/tasks.md) own implementation sequencing. Formal specifications and the command reference currently remain in Chinese.

## 1. Positioning: an independent tool and PruneX foundation

DiskGraph is a Rust engine for relationships among files, directories, applications, projects, processes and generation rules. Its target capabilities include persistent indexes, history, bounded queries and optional controlled operations. It can be installed independently for agents or embedded beneath PruneX.

- `PruneX ≈ disktree-app`: user experience, interaction, product workflows and model orchestration.
- `DiskGraph shared engine ≈ disktree-core`: shared lower-level capabilities, not feature equivalence.
- DiskGraph is the entire project; `diskgraph-core` is only its model/contract crate.
- `diskgraph-mcp` is a Rust protocol adapter, not a second scanner or graph store.
- A prebuilt binary can include its scanning dependency; end users need not install disktree-app, PruneX or a Rust toolchain.

The engine should answer what consumes space, who owns it, what changed, what uses it, whether it is rebuildable, what an operation might affect, and how an authorized operation can be verified.

Efficiency comes from index reuse across sessions, compact relational context, stable pagination and targeted updates. Wrapping ls/du/find alone is insufficient. Cold indexing, freshness and accuracy count alongside warm-query latency.

**Change from the initial design: queries remain read-only, but execution no longer belongs exclusively to PruneX.** Optional `diskgraph-ops` enables authorized actions on headless servers. PruneX supplies a trusted review experience; the server validates approvals, rechecks live resources and records effects.

The core does not require AgentScope or a model. Planned PruneX macOS/iOS orchestration uses Swift and AgentScope-Swift; Android uses Kotlin and AgentScope-Kotlin. Lite cloud models and Pro local models are PruneX choices, not sources of authority inside DiskGraph.

## 2. Existing foundation and gaps

| Area | Current source evidence | Target |
| :--- | :--- | :--- |
| Crates | core/store/disktree/ffi | Add engine/collectors/ops/cli/mcp |
| Models | DiskSnapshot, DiskNode, EvidenceEdge with textual subject | Lossless locators, typed entities, versioned evidence and coverage |
| Scan | Native-path bridge; empty evidence; absent FileIdentity; lossy conversion | Lossless identity, scoped errors, project/app/process collectors |
| Sizes | direct/subtree bytes under one selected measurement | Separate apparent/allocated sizes and unknown values, not guaranteed reclaim |
| History | v1 growth requires complete scans, identical settings and known equal volumes | Explicit comparability and partial coverage; no default rename inference |
| Storage | Immutable transactional SQLite v1 snapshots and paging | Migrations, graph revisions, control records, retention and quotas |
| Queries | top/children/growth/explain/candidates | 29 command families and shared service methods |
| Adapters | Read-only UniFFI JSON v1 | CLI, three MCP transports and versioned native APIs |
| Platforms | Desktop native scan entry points; no mobile URI scanner | Capability-based releases and restricted mobile providers |

Current candidates require explicit Rebuildable evidence; classification hints alone do not grant eligibility. Directory modification time is aggregated from the subtree, not necessarily the directory's own mtime. The Windows bridge lacks volume identity.

Evidence: [models](../crates/diskgraph-core/src/model.rs), [queries](../crates/diskgraph-core/src/query.rs), [store](../crates/diskgraph-store/src/lib.rs), [bridge](../crates/diskgraph-disktree/src/lib.rs), [FFI](../crates/diskgraph-ffi/src/lib.rs). On 2026-09-28, nine existing unit tests, fmt and Clippy passed locally on macOS with Rust 1.98.1. This does not verify remote CI, the declared Rust 1.97 minimum, installation packages or mobile devices.

## 3. Overall structure

### Current implementation call paths (2026-10-01)

```mermaid
flowchart TD
    CLI["CLI / TUI / HTML"] --> E["Engine<br/>Authorization, jobs, queries, content inspection"]
    MCP["MCP<br/>stdio / HTTP / legacy SSE"] --> S["McpService<br/>Tool dispatch"]
    S --> E
    E --> SC["disktree scanner"]
    SC --> CV["Tree conversion<br/>Identity and metadata enrichment"]
    CV --> ST["Staging → revision publication"]
    ST --> G[("Graph database<br/>Snapshots, nodes, relations")]
    E --> C[("Control database<br/>Scopes, permissions, jobs, operation records")]
    OPS["Ops<br/>Planning, approval, execution, recovery"] --> E
    FFI["Swift / Kotlin FFI"] --> E
    E -. "Legacy FFI: narrow read after authorization" .-> G
```

Legacy FFI signatures remain compatible. Engine now checks actual snapshot/revision ownership and live authorization before opening a read connection. Raw store APIs remain trusted internal compatibility entry points. The hardening record below separates implementation from platform acceptance.

### Target architecture

```mermaid
flowchart TB
    AG["Local or remote agents"] --> MCP["diskgraph-mcp<br/>stdio / Streamable HTTP / legacy SSE"]
    SH["Terminal users and scripts"] --> CLI["diskgraph-cli"]
    UI["PruneX Swift / Kotlin"] --> FFI["diskgraph-ffi"]
    MCP --> AUTH["Shared service boundary<br/>identity, scope, budget, policy"]
    CLI --> AUTH
    FFI --> AUTH
    AUTH --> ENG["diskgraph-engine<br/>jobs, indexes, queries"]
    AUTH --> OPS["diskgraph-ops<br/>plans, approval validation, execution"]
    ENG --> SCAN["disktree / platform providers"]
    ENG --> COL["Project, app, process, rule collectors"]
    ENG --> STORE["diskgraph-store"]
    OPS --> STORE
    OPS --> OS["Platform actions / ecosystem adapters"]
    OPS -->|refresh and measure| ENG
    STORE --> GRAPH[("diskgraph.sqlite<br/>rebuildable index")]
    STORE --> CTRL[("diskgraph-control.sqlite<br/>policy, operations, recovery, audit")]
    UI --> BUSINESS["GRDB / Room"]
    BUSINESS --> APPDB[("prunex.sqlite<br/>product data")]
```

Arrows show runtime calls. Compile-time dependencies use core ports. CLI, MCP and FFI are peers: MCP does not shell out to the CLI, and PruneX does not require an MCP subprocess.

| Module | Status | Responsibility |
| :--- | :--- | :--- |
| diskgraph-core | Existing, extend | Models, IDs, relations, DTOs and ports; no SQLite/UI dependency |
| diskgraph-store | Existing, extend | Schemas, transactions, migrations, paging and retention for Rust-owned stores |
| diskgraph-disktree | Existing, extend | Pinned scanning bridge; no upstream removal calls |
| diskgraph-engine | Planned | Scan/collector jobs, revision publication, queries and budgets |
| diskgraph-collectors | Planned | Project/app/process/rule evidence and validity, not model instructions |
| diskgraph-ops | Planned | Immutable plans, approval validation, execution, idempotency and recovery |
| diskgraph-cli | Planned | Local/remote commands, service bootstrap and client registration |
| diskgraph-mcp | Planned | Three transports, tool schemas, identity context and service adaptation |
| diskgraph-ffi | Existing, reorganize | Swift/Kotlin APIs, job handles, background work and cancellation |
| Platform providers | Modules or host code | NativeFs, Android URIs, iOS documents and capability limits |

ops requests refresh through a port; engine and ops must not form a dependency cycle. Split additional platform crates only when packaging or maintenance warrants it.

## 4. Reuse, CodeGraph and operating-system commands

Keep `disktree-core` pinned behind a bridge. A small source-file count does not remove platform, safety and upstream maintenance costs. Consider patches, a fork or vendoring only when demonstrated requirements cannot be met by adaptation, preserving provenance, licenses and a change ledger.

| Reference | Reuse | Do not copy |
| :--- | :--- | :--- |
| disktree | Scanning, size handling, hard links, classification | Classification as approval or removal authorization |
| PureMac | Application IDs, container association and protection experience | Deleting every name match |
| mac-cleanup-sh | Ecosystem categories and rule examples | Full cleanup scripts or arbitrary Shell |
| CodeGraph | Persistent indexing, explore, exact relations, tool profiles, invalidation | AST semantics, code ignore lists or advertised efficiency numbers |

target, node_modules, hidden directories and caches are valuable scan inputs. A compact default MCP tool list is not a four-capability ceiling: service APIs and tool-list presentation are separate choices.

Basic enumeration, find/du/stat/df/read/move/copy-like behavior uses Rust/platform APIs, not mandatory system commands. Git, lsof-like observations, Cargo and Docker are optional adapters. Missing dependencies disable that capability; they do not permit raw-directory deletion.

The [reference study](reference-study.md) preserves historical investigation. Its earlier test results, upstream state and execution boundaries are not current acceptance evidence; this OpenSpec change governs newer requirements.

## 5. Data: an evidence graph over a directory tree

| Object | Meaning |
| :--- | :--- |
| DiskSnapshot | Root/volume/provider, options, time window, coverage and errors; not an atomic filesystem snapshot |
| DiskNode / Resource | Snapshot-local ID, parent, lossless locator, type, sizes and timestamps |
| CollectorRun | Method/rule versions, fingerprints, access coverage, time and failures |
| GraphRevision | Immutable composition of a file snapshot and selected collector runs |
| EvidenceEdge / Record | Typed assertions, supporting/contradicting evidence, provenance, freshness and dependencies |

ResourceRef contains server, scope, revision and node. It is a reference, not an access token. Paths, volume/inode IDs and hashes are not permanent identities on their own. Display strings and operational locators are separate.

```mermaid
flowchart LR
    MAN["Cargo.toml"] -->|declares| PROJ["Project"]
    OUT["target directory"] -->|owned_by_project| PROJ
    OUT -->|rebuildable_by| RULE["BuildRecipe"]
    OUT -->|contains| FILE["Output file"]
    FILE -->|used_by_process| PROC["Process"]
    CACHE["Cache directory"] -->|owned_by_application| APP["Application"]
    FILE -->|protected_by| POLICY["ProtectionPolicy"]
```

A relationship is an assertion, not permission. Ownership does not imply deletability; rebuildability does not guarantee acceptable reconstruction costs; lack of observed use does not prove inactivity. Multiple owners and conflicts remain visible. Confidence is not a probability or an authorization score.

Freshness is fresh/stale/invalidated/unknown, with conflict recorded independently. Expired use evidence does not become “idle”; unavailable protection does not revoke protection. Candidates report eligible_for_review/blocked/unknown, never safe_to_delete.

Unknown sizes are not zero. Hard links, shared blocks, sparse files, system snapshots and open handles can change reclaim behavior. Observed size and measured free-space changes remain separate.

## 6. Storage ownership and server locality

Preserve the Rust graph/PruneX business-data separation, adding a Rust control store for standalone execution.

| Data | Owner | Lifecycle |
| :--- | :--- | :--- |
| diskgraph.sqlite | Rust diskgraph-store | Rebuildable file/relationship index and retained history |
| diskgraph-control.sqlite | Rust diskgraph-store | Scopes, policies, jobs, plans, approvals, operations, recovery and audit; survives index rebuild |
| prunex.sqlite | PruneX GRDB / Room | Sessions, preferences, UI and workflow projections; not remote execution authority |

Swift/Kotlin do not independently implement DiskGraph schema management. Native SQLite versions, symbols, linking and connection lifetimes still require validation; different database filenames do not remove linking conflicts.

Scanning, databases and effects remain on the server containing the resources. A local agent accesses authorized results through the protocol, without mounting SQLite or treating remote paths as local. Server recovery does not depend on a PruneX installation.

## 7. Complete command surface and three MCP forms

The target has **29 commands or command families**:

| Category | Commands |
| :--- | :--- |
| Scopes and indexes | scope, index, sync, status, snapshots |
| History | changes, growth |
| Graph queries | explore, search, node, children, top, related, explain, impact, candidates |
| Content | duplicates, read |
| Controlled file actions | move, copy, trash, restore, purge |
| Execution governance | plan, apply, operations |
| Deployment/diagnostics | serve, install, doctor |

CLI/MCP/FFI share DTOs and services. A minimal read profile may expose common tools, while complete exact queries remain configurable. manage/write permissions are separate. serve/install configure the local host and are not remote tools for modifying an agent's machine.

| Transport | Use | Boundary |
| :--- | :--- | :--- |
| stdio | A local host launches a process | Protocol-only stdout; local identity still obeys scope policy |
| Streamable HTTP | Local agents query a server | Authentication, encryption, Origin checks, budgets and version compatibility |
| legacy HTTP+SSE | Older clients | Isolated adapter, disabled by default, actual legacy-client tests |

An SSE response inside Streamable HTTP does not establish legacy HTTP+SSE support. SDK and compatibility paths need separate validation; one feature flag must not be described as universal compatibility.

Separate metadata, content, index administration, scope administration, individual file actions and approval issuance. Remote subjects access preregistered scopes; queries never silently index the whole machine. Do not expose arbitrary SQL/Shell, automatic privilege escalation or unlimited path access.

## 8. Operations and honest recovery

```mermaid
sequenceDiagram
    participant A as Agent or PruneX
    participant D as DiskGraph service
    participant U as Trusted review channel
    participant O as Controlled executor
    participant F as Local filesystem
    A->>D: candidates / explain / impact
    D-->>A: IDs, evidence, unknowns, coverage
    A->>D: move / trash creates a plan
    D-->>U: Immutable targets, risks, recovery, digest
    U->>D: Approval bound to principal and plan
    A->>D: apply(plan_id, approval_ref, idempotency_key)
    D->>O: Validate approval and persist intent
    O->>F: Recheck identity, scope, use and conflicts
    alt Changed, denied or unknown
        O-->>D: stale_plan / blocked
    else Allowed and verifiable
        O->>F: Platform action
        O-->>D: Per-item result, recovery reference, measurements
        D->>D: Persist result and refresh affected index
    end
    D-->>A: Operation ID and actual state
```

- move/copy/trash/restore/purge create plans by default; only apply can execute.
- approved=true, free text, a TTY or --yes cannot let the agent approve itself. Approval comes from a trusted channel or a bounded administrator policy.
- Plans bind directory membership. Revalidate new protected descendants, mount changes, identity, occupancy coverage and destination conflicts.
- Same-volume moves and cross-volume copy/verify/publish/delete-source are different workflows, not one global atomic transaction.
- Trash failure never falls back to permanent deletion. Restore never silently overwrites. Purge is separately authorized and cannot be undone by a graph snapshot.
- Same-volume trash/quarantine does not generally free those bytes. Report processed bytes, retained bytes and measured volume free space separately.
- Persist intent first; use idempotency; uncertain post-crash outcomes become needs_attention. Cancellation does not undo completed items.
- Rebuilding the index must not delete recovery records, nor may those records be discarded to create space.

Cargo/Docker adapters use exact project/object plans and domain APIs. Do not delete Docker VM/volume directories as ordinary caches or broaden one approved object into global prune.

## 9. Cross-platform boundaries and PruneX

| Platform | Input and boundary | Release evidence |
| :--- | :--- | :--- |
| macOS | Native paths, permissions, placeholders, trash capabilities | First standalone package and real Swift calls |
| Linux | Server-local scopes, low privilege and platform recovery | Remote transports and restart/recovery |
| Windows | Native encoding, volume/file IDs, reparse points and locks | Real-machine queries and per-action verification |
| Android | Kotlin SAF / applicable media provider URIs | Device grants, unknown sizes, paging and revocation |
| iOS | App-owned/user-selected documents and scoped access | Device coordination, revocation and cancellation |

Mobile restrictions exist in P0 contracts, not as a late porting fix. Android does not promise access to every app's private data; iOS does not promise whole-device cleanup or cross-app uninstall.

Target FFI supports background jobs, paging, cancellation and structured errors. PruneX owns views, intent and AgentScope orchestration. Cloud models receive no unauthorized content; model output cannot change observed facts or mint approval.

## 10. Delivery stages and acceptance

| Phase | Dependencies | Exit gate |
| :--- | :--- | :--- |
| P0 Baseline/contracts | None | Existing behavior, lossless identity, authorization and compatibility fixtures |
| P1 Snapshots/storage | P0 | Scopes, reliable scans/migrations, capacity and jobs |
| P2 Queries/CLI | P1 | Project evidence, history, exact read-only queries and budgets |
| P3 Local agents | P2 | stdio, private macOS package, two real hosts |
| P4 Remote read-only | P3 | Linux, identity, HTTP/legacy SSE, disconnect and isolation |
| P5 Recoverable execution | P4 | Plans/approval, same-volume actions, trash/restore and failure gates |
| P6 High-risk actions | P5 | Cross-volume work, purge, Cargo/Docker and measured space |
| P7 Content/desktop | P2; write verification also P5/P6 | read/duplicates, richer evidence, watch, Windows |
| P8 PruneX embedding | P3; write UI also P5 | FFI, database coexistence, trusted review |
| P9 Restricted mobile | P8 | Android/iOS device provider validation |
| P10 Full private delivery | P6/P7/P9 | Commands/transports/platforms, efficiency and upgrade drills |

Completing P3 yields an independent read-only product without waiting for mobile. Linux is brought forward in P4 for server use. P5 must implement its own minimum live safety checks rather than wait for P7's richer collectors.

Use the same models/data and repeated trials to measure correctness, tool calls, returned bytes/tokens, retained context, cold/warm/sync time, peak memory and DB/WAL/log cost. Test denial, races, crashes, full disks, concurrency, partial completion and recovery conflicts. Compilation, HTTP 200 and tool discovery are not complete acceptance.

SDK versions, TTL/capacity defaults, signing identities and device resources need phase-specific evidence. Publication, production deployment and actual cleanup require separate authorization.

## 11. Processes, concurrency and lifecycle

Current FFI functions are synchronous, some queries load whole snapshots, and no service scheduler exists yet.

| Target mode | Process/state owner | Shutdown/failure contract |
| :--- | :--- | :--- |
| Local CLI / stdio | CLI or host-launched process; shared index within an authorization domain | Exiting a connection does not delete shared indexes; leases coordinate duplicate requests |
| HTTP service | Long-running low-privilege process on the resource server | Disconnect does not imply cancellation; original job/operation remains queryable |
| Embedded PruneX | Rust engine inside the UI process; host owns native grants | No blocking scan on the UI thread; release handles and provider access |

Use bounded queues and scan/hash/copy pools, coordinated SQLite writes and independent query read transactions. Keep blocking work off the async transport loop. Coalesce scope scans and serialize overlapping source/destination/ancestor actions. Thread counts and queue limits require benchmarks.

```mermaid
stateDiagram-v2
    [*] --> starting
    starting --> ready: Configuration, database and access checks pass
    starting --> failed: Required preconditions fail
    ready --> degraded: Optional dependency unavailable
    degraded --> ready: Recovered and revalidated
    ready --> draining: Shutdown or maintenance
    degraded --> draining: Shutdown or maintenance
    draining --> stopped: Stop admission, record state, release resources
    failed --> [*]
    stopped --> [*]
```

Drain stops admission, cancels or waits for safely interruptible steps, records unfinished work and releases resources. A deadline cannot pretend to roll back all effects. Uncertain effects retain needs_attention; reconcile control records and real resources before re-enabling affected write capabilities.

## 12. Configuration and security controls

There is no standalone configuration loader today; callers pass library arguments. The proposed configuration in the technical design is not a runnable format.

| Configuration domain | Authority | Override boundary |
| :--- | :--- | :--- |
| Listener, database location, transports, budgets | Validated deployment configuration | Requests may tighten budgets, never expand scope roots |
| Scopes, policy, approval, revocation | Control store and trusted policy sources | Ordinary clients/models cannot write them directly |
| Secrets/native document access | System secret store or trusted host | No printable secrets in logs or graph data |
| Tool presentation profile | Deployment configuration | Presentation never replaces per-request authorization |

Treat names, manifests, tool results, requests and model outputs as untrusted. Inject authenticated identity server-side. Reject cross-scope references, forged approval, path/argument injection and unbounded content export.

In-process Rust adapters are trusted code, not a claimed plugin sandbox. Start with reviewed built-in providers/collectors. Dynamic untrusted plugins, arbitrary remote Shell, direct cross-machine moves, SaaS billing and embedded RAG are outside scope.

## 13. Decisions, alternatives and reversal conditions

[OpenSpec D1–D15](../openspec/changes/implement-diskgraph-platform/design.md) remains the decision record. This table is an explanatory summary, not a second approval process.

| Decision | Choice/rationale | Alternative/cost | Revisit when |
| :--- | :--- | :--- | :--- |
| D1 Shared Rust service | Consistent semantics across adapters; separate ops | Per-host implementations drift | Proven ABI or isolation requirements change |
| D2 Pinned scanner | Reuse fixes behind a bridge | Copying transfers maintenance | Lossless/platform needs require upstream changes |
| D5 Separate graph/control stores | Rebuilds retain recovery authority | More backup and reconciliation work | A migration/recovery design is demonstrated |
| D7 Rich API, configurable presentation | Exact scripting and smaller default tool lists | explore-only limits precision | Comparative client evaluations justify change |
| D8 Separate legacy adapter | Compatibility without mixing protocol paths | Legacy maintenance cost | User approves dropping legacy clients |
| D10 Trusted approval/revalidation | Prevent model self-authorization | Interaction and latency cost | Only add approved bounded policies, not remove safeguards |
| D14 Platform capabilities | Honest URI/sandbox semantics | No universal whole-device promise | Actual OS/provider changes pass acceptance |

## 14. Observability, operations and budgets

| Signal | Target fields/metrics | Constraint/action |
| :--- | :--- | :--- |
| Logs | request/job/operation ID, component, status, code, duration | Bounded and redacted; stdio logs on stderr; no bodies/tokens |
| Metrics | Query latency, queue depth, denial rate, scanned nodes, DB/WAL/staging | Bounded component/action/result labels, not paths or user/file IDs |
| Audit | Principal, policy/approval digest, per-item intent/result/recovery | Not deleted by normal log rotation or index rebuild |
| Readiness | ready/degraded/draining, schema and required dependencies | Process liveness or HTTP 200 alone does not mean safe writes |
| doctor | Capabilities, permissions, dependencies, capacity, backlog | Read-only; no auto-install, escalation, unlock or cleanup |

Telemetry failure cannot produce unlimited buffers. Ordinary telemetry may be sampled with loss reported; execution intents and recovery audit cannot follow a lossy policy. Stop new filesystem effects when authoritative recording fails.

Plan memory as baseline + active tasks × task budget + bounded cache. Count graph, WAL, backups, staging and quarantine storage. Depth 2, 100 nodes, 300 edges, 64 KiB and a one-second query deadline are proposed starting limits, not measured SLAs. RTO/RPO, worker counts and retention days await deployment drills.

## 15. Risks, ownership and unresolved verification

Owners are maintenance roles, not invented people or calendar commitments. P0–P10 are delivery phases, not template priorities or release versions.

| Risk/gap | Responsible role | Exit evidence/phase |
| :--- | :--- | :--- |
| Lossy paths, Windows volume IDs, whole-graph v1 queries | core/scanner/store | Lossless fixtures, platform and capacity tests; P1/P2/P7 |
| MCP and legacy-client differences | Protocol maintainer | Pinned SDK/client matrix and three actual transports; P3/P4 |
| Races, incomplete occupancy, crash reconciliation | ops/security | Negative/full-disk/disconnect/recovery tests; P5/P6 |
| SQLite/GRDB/Room and mobile permissions | Native integration | Actual host loading and device revoke/cancel; P8/P9 |
| Signing, notarization, devices, budget defaults | Release/testing | Private artifacts and repeated benchmarks before P10 |

This document adapts the complete architecture template with runtime, extension, agent-tool safety and observability/control-plane profiles. Commerce, SaaS, brokers, plugin marketplaces, RAG/training and embedded hardware examples are excluded. AgentScope/Lite/Pro describe planned PruneX responsibilities, not existing DiskGraph dependencies. No formal requirements or task completion marks were added.

## 16. References and traceability

- [Technical design](DiskGraph-Technical-Design.md): schemas, migration, protocols, state, guards and tests.
- [Command reference](command-reference.md): C01–C29 CLI/MCP/permission/phase mapping, currently Chinese.
- [OpenSpec decisions](../openspec/changes/implement-diskgraph-platform/design.md): D1–D15, alternatives, risks and migration.
- [Implementation tasks](../openspec/changes/implement-diskgraph-platform/tasks.md): staged work and acceptance.
- References: [DiskTree](https://github.com/tobi/disktree), [CodeGraph CLI](https://github.com/colbymchenry/codegraph#cli-reference), [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk#transports).

---

**Document version**: 1.1\
**Created**: 2026-09-28\
**Updated**: 2026-09-28\
**Status**: pending review; design completeness is not implementation completion.

## 2026-10-01 hardening implementation boundary

The following flow is implemented in this change. Earlier sections marked Target remain subject to platform acceptance.

```mermaid
sequenceDiagram
    participant R as HTTP / SSE request
    participant A as Origin + token validation
    participant E as Engine request authorization
    participant C as Control database
    participant G as Independent graph reader
    R->>A: Request and credentials
    A->>E: Immutable principal, capabilities, transport, expiry
    E->>G: Resolve actual revision server / scope
    E->>C: Read live policy and revocation
    C-->>E: Database grants
    E->>E: Token capabilities ∩ live grants
    E->>G: Deadline-bound narrow reads / ordered merge
    G-->>R: Data and truncation diagnostics
```

A serial graph writer handles mutations. Conditional job claims increment fencing; a control transaction validates lease, fence, scope and live IndexWrite before staging batches, revision publication and collector writes. Expired claims rescan in a new staging namespace. The upstream scanner pin, source and digests remain unchanged. Conversion traverses iteratively; publication generates formal rows from staging.

SQLite consistent backups, including committed WAL, precede migrations in `migration_backups/`. Graph schema 6 adds ownership and normalized search; schema 7 adds sparse unknown-size, relation-page and ordered-path indexes; schema 8 adds review-candidate size and evidence-relation indexes. Schema 9 maintains exact snapshot/directory counts and size-prefix counts in each publication transaction; migration backfills them atomically. Snapshot writer markers reject already-open obsolete writers, and missing count metadata fails closed. Known/unknown pages retain legacy JSON fallback semantics and matching partial indexes. Explicit OFFSET still costs O(offset+page). Aggregation increases storage and publication/migration costs; measured backup/WAL/temporary-file costs are in the [full-platform record](production-readiness-full-platform-2026-10-02.md). Candidate selection and impact traversal use request-scoped readers with deadlines and explicit truncation. Control schema 4 adds leases/fencing, and schema 5 persists cancellation. Old running jobs wait for heartbeat + 30 seconds instead of being stolen at startup. Graph WAL/NORMAL preserves transaction consistency but may lose recent reconstructible index commits on power loss. Control FULL preserves the required operation-record durability. There is still no cross-database atomic transaction guarantee.

```mermaid
flowchart LR
    X["Cancel or revoke"] --> C[("Control DB: durable intent")]
    C --> F{"Lease + fence + live grant"}
    F -- valid --> S["Write current staging namespace"]
    F -- cancelled / stale --> R["Stop; leave old revision intact"]
    S --> F2{"Publication fence"}
    F2 -- valid --> P["Atomic revision publication"]
    F2 -- cancelled / stale --> R
    O["Approved file plan"] --> V["Live auth + source fingerprint"]
    V --> A["Immediate transaction: path + file identity claim"]
    A --> H["Handle-bound, no-replace operation"]
```

FFI derives its control-data realm from the lossless graph database path; ambiguous legacy shared control data is rejected. Operation plans require a complete digest and fresh source evidence, so old plans must be recreated. Positive-target candidate selection still loads a full revision; relation impact is paged but opens a read connection per page/direction. These remain explicit query-cost limits.

See the [acceptance record](security-performance-hardening-2026-10-01.md) for compatibility, fidelity checks, cancellation overshoot, physical retention costs and local measurements. CLI/MCP dangerous tools remain closed; Linux/Windows native writes and strict scanner RSS bounds are not accepted capabilities.
