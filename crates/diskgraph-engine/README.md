# diskgraph-engine

Job orchestration and the authorized service layer of
[DiskGraph](https://github.com/loong10k/diskgraph): scope registration,
durable index jobs with leases, staging with atomic publication, capacity
watermarks, and the tree/read narrow paths.

## Install

```toml
[dependencies]
diskgraph-engine = "0.2"
```

## What it enforces

- **Scopes** are registered against a canonical root and revocable; a
  revoked scope yields no plan and accepts no job.
- **Jobs** are leased with owner fencing: a queued or running job merges
  rather than duplicating, and a terminal job is never re-claimed.
- **Budgets** stop a walk for a named reason (`NodeLimit`, `StagingLimit`,
  `TimeLimit`, `Cancelled`) instead of returning less data silently, and a
  stopped walk publishes nothing.
- **Capacity** watermarks refuse new work past a threshold and never
  delete anything to make room.
- **Cancellation** is observed between observation boundaries, so a
  cancelled scan leaves no half-published revision.

## Bounded content reads

`read_bounded` and `digest_bounded` run under a separate `content:read`
grant, a byte budget, and an identity contract: observed identity/version
changes report `Unstable` and void the digest. This is not an atomic snapshot.

Windows content acquisition uses a local absolute drive path and one
component at a time relative to retained directory handles. It requests
attributes before data, exposes placeholder attributes on the calling
thread, and refuses reparse/offline/recall objects before reading. The
read retains a data handle denying ordinary write/delete sharing and checks the full 128-bit
file ID, volume ID, length and native write/change timestamps. ADS,
parent traversal, UNC/device paths and unknown volume/identity support
are refused. The registered root's component spelling is required;
lexical checks do not guess case/alias equivalence. Attribute acquisition
does not freeze new writers: changes are rejected as conflict before
data access. Native timestamp units do not guarantee filesystem precision.

Sharing excludes ordinary write/delete handles, not every writable
mapping, kernel or filter mutation. Version checks are not an atomic
content snapshot. Windows `FILE_OPEN_NO_RECALL` constrains opening;
real provider no-download acceptance remains separate, as does Linux
placeholder protection. The optional thread API (Windows 10 1709+) is
dynamically resolved and missing support refuses inspection. Deadline and
cancellation checks are cooperative and cannot preempt synchronous native
opens or reads. Native evidence covers CI NTFS fixtures, not every local
filesystem. See the [full-platform acceptance record](../../docs/production-readiness-full-platform-2026-10-02.md).

## Source boundaries / 源码边界

`lib.rs` only declares modules and preserves public exports. `Engine` remains
the sole owner of its connections, cancellation flags and progress state;
responsibility modules supply implementations on that same object. No new
service or repository owners are introduced. Public `content`, `live_evidence`
and `verify` paths remain compatible through declaration/reexport façades.

| Responsibility / 职责 | Real implementation / 实际实现 |
| :--- | :--- |
| State, startup, capacity / 状态、初始化、容量 | `engine.rs`, `engine_startup.rs`, `engine_capacity.rs` |
| Scopes, policy, authorization / 范围、策略、授权 | `scope_service.rs`, `policy_service.rs`, `revision_authorization.rs` |
| Jobs, scan, publication / 任务、扫描、发布 | `scan_jobs.rs`, `scan_execution.rs`, `runner.rs` |
| Reader, queries, history / 读连接、查询、历史 | `revision_reader.rs`, `revision_queries.rs`, `relation_access.rs`, `relation_queries.rs`, `tree_queries.rs`, `revision_history.rs`, `revision_comparison.rs`, `sync_plan.rs` |
| Retention / 历史回收 | `snapshot_retention.rs` |
| Content, collectors, live evidence, verification / 内容、采集、实时证据、核验 | `content/`, `collectors/`, `live_evidence/`, `queries/`, `verify/` |

```mermaid
flowchart TD
    API["CLI / MCP / FFI / Ops"] --> AUTH["Scope / policy / revision authorization"]
    AUTH --> JOB["Jobs / fenced scan / publication"]
    AUTH --> READ["Queries / history / content"]
    JOB --> E["Single Engine<br/>connections, cancellation, progress"]
    READ --> E
    E --> G[("Graph writer + independent readers")]
    E --> C[("Control authority")]
```

授权入口与可信内部兼容入口仍有明确区别：`revision_reader`、原始窄读方法和
`control_store` 不自行授权请求，远程适配器必须先经过资源授权。发布和历史回收
保留 graph→control 锁顺序；已有 control guard 使用 `require_with_control`，避免重入。
类型（含私有记录、trait 和 alias）独立文件，公共方法中文注释说明参数、返回和实际原生来源。

`tests/source_layout.rs` parses all platform modules and checks entry façades,
one object per production file, fewer than 500 physical lines, snake_case
paths, Chinese contracts, no wildcard imports or placeholder/empty functions,
and no unmounted source files. It supplements behavioral and native-platform
tests; source organization alone does not prove production readiness.


`sample_process_usage` preserves visible positive observations with `Partial`
coverage: successful exit does not verify permission scope or PID start identity.
Byte parsing rejects malformed records; potentially escaped/annotated path names
remain unknown instead of being guessed into native identities. Empty-query
`Full` is only a compatibility result for zero objects. `ProbeLimits` and the
additive `sample_process_usage_bounded` / `sample_git_bounded` entries share one
absolute deadline, cancellation flag and cumulative stdout/stderr limit across
the entire sample. Existing entries default to 15 seconds and 1 MiB. Fixed pipe
buffers still verify actual EOF at the exact limit; an extra byte, abnormal exit
or failed cleanup cannot become a completed observation. Cleanup can exceed the
cooperative deadline and does not establish a strict RSS or latency ceiling.

Unix retains the leader until group termination and reaping. Hosts must not
auto-reap or independently wait for this child; those conditions fail closed.
Ordinary descendants remain in the owned group; descendants that actively leave
it are outside this containment. Windows requires Windows 10+, a trusted absolute
`.exe` or simple executable name in explicit absolute PATH entries, and an
absolute project directory; relative program paths and scripts are refused.
Native execution uses creation-time Job/standard-handle attributes and owned
overlapped pipes. Job accounting has a separate one-second observation window;
unknown or nonzero state refuses completed evidence. An externally held process
handle does not itself imply a nonzero Job count; the actual query decides. Leader waits and safe pending-I/O completion
can exceed that window. Its final native acceptance is recorded in
the [full-platform evidence](../../docs/production-readiness-full-platform-2026-10-02.md).
Git sampling requires Git 2.46+ and distinguishes missing references from broken
references and failed commands. Unborn repositories retain actual dirty files;
NUL status records count each untracked file and one entry per rename/copy.
Invalid OIDs and commit counts fail the sample. Existing stash history requires
the `files` reference backend: bounded raw reflog reads, retained commit checks
and exact reverse-order comparison detect records silently omitted by Git.
The common metadata directory is located by Git; its fixed log suffix is opened
without following links. This location is not scope or configuration isolation.
Legitimate drop/delete/expiry and absent logs preserve visible-list semantics.
Both raw reads share the sample byte/deadline/cancellation budget; reftable stash
enumeration is explicitly unsupported. Terminal reference/version checks detect
changes but do not form an atomic Git snapshot. Native acceptance for this
semantic increment is recorded separately in the full-platform evidence.
Git configuration isolation and offline/read-only execution remain outstanding.

占用采样保留正向观察并报告 partial；空结果、转义/标注名称或无法确认的身份不说明无人使用。
原生字节键不等于文件句柄/PID 启动身份。新增有界采样入口共用整次期限、取消和
stdout/stderr 累计字节，兼容入口默认 15 秒/1 MiB；预算耗尽仍核验实际 EOF，
异常退出与清理失败不能变成完整样本。清理可能超过协作期限，不承诺严格 RSS/SLA。
Unix 宿主不得 auto-reap 或外部回收该 child，主动脱离组的后代不受组约束。
Windows 要求 Windows 10+、受信 .exe 和绝对项目目录，拒绝相对程序路径及脚本；
平台原生验收见上述记录。Git 采样要求 2.46+，明确区分缺失、损坏及执行失败；
unborn 仍报告实际修改，NUL 原生记录按文件及 rename/copy 计数，非法 OID/数量报错。
stash 存在时仅支持可核验的 files 后端：共用整次预算读取原始日志、核验留存 commit
及完整逆序列表；reftable 明确拒绝。日志路径来自 Git common 根与固定后缀，
合法 drop/delete/expiry 和明确缺日志保留可见列表语义。末段版本复核不是 Git 原子快照。
Git 配置隔离、离线/只读执行仍待完成，此语义增量的原生验收单独记录。

## License

MIT
