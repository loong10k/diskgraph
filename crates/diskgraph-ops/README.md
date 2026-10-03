# diskgraph-ops

Reversible, approval-gated file operations for
[DiskGraph](https://github.com/loong10k/diskgraph): immutable plans bound
to digests, trusted approvals, live revalidation before anything moves, and
durable intent records that make a crash recoverable instead of replayable.

## Install

```toml
[dependencies]
diskgraph-ops = "0.2"
```

## The shape of an operation

1. **Plan** — resolve node ids to live paths, identities, and sizes; drop
   parent/child overlaps so the same bytes are never counted twice; write
   a digest over the exact object set. Planning reads source evidence and records
   the plan without modifying source files.
2. **Approve** — a trusted surface (a review console, an admin policy)
   mints an approval bound to the plan's digest, principal, and action. An
   agent can never assert its own approval; a purge additionally requires
   the configured purge authority.
3. **Apply** — revalidate every precondition (identity, scope boundary,
   symlink components, occupancy), persist the intent, then act, then
   record the result. A crash between intent and result parks the
   operation as `NeedsAttention` instead of replaying blindly.

## What it does

- same-volume and cross-volume moves, copies, and verified staging
- quarantine + restore for trash, with recovery records
- purge behind its own authority, with no recovery by design
- Cargo and Docker specialist adapters behind an allow-list, driven
  through a sandboxed subprocess runner

## Source boundaries / 源码边界

The entry preserves explicit public exports. Each distinct production type has
its own file; the existing `specialist` and `docker` module paths remain public.
The split keeps the original method bodies, transaction and lock order, wire
formats and explicit failure cleanup. It adds no runtime service or state owner.

入口保留明确公开导出；每个生产对象独立文件，`specialist` 与 `docker`
旧模块路径保持。拆分保留原方法体、事务/锁顺序、序列化及显式失败清理，
没有增加服务层或状态持有者。

| Responsibility / 职责 | Source / 源码 |
| --- | --- |
| Plans and trusted approvals / 计划与可信批准 | `plan_builder.rs`, `approval_issuer.rs`, `plan_digest.rs` |
| One executor and its existing methods / 唯一执行状态与原方法 | `executor.rs`, `executor_apply.rs`, `executor_validation.rs`, `executor_perform.rs`, `executor_transfer.rs`, `executor_paths.rs` |
| Transfer owner and staging methods / 传输持有者与 staging 方法 | `cross_volume_copy.rs`, `cross_volume_staging.rs` |
| Live authority, source versions and native paths / 实时授权、源版本与原生路径 | `ops_authorization.rs`, `source_evidence.rs`, `path_codec.rs`, `path_revalidation.rs`, `bound_path.rs` |
| Operation queries and capacity observations / 操作查询与容量观察 | `operation_queries.rs`, `volume_capacity.rs`, `scope_refresh.rs` |
| Existing adapter contracts / 原生态适配契约 | `specialist/`, `docker/` |

The AST layout gate walks platform branches and rejects entry definitions,
mixed types, large production files, wildcard imports and placeholders. Its only
empty-body exception is the preexisting zero-resource `CrossVolumeCopy::discard`
on unsupported platforms; it is not implemented write support. Native file
operations still require their own platform/fidelity acceptance, and public
CLI/MCP dangerous tools remain disabled.

AST 门禁检查各平台分支，拒绝入口定义、多对象混放、大生产文件、通配导入及
占位实现。唯一空方法例外是原有不支持平台的零资源 `CrossVolumeCopy::discard`，
不表示写能力已实现。原生文件操作仍须平台及保真验收；CLI/MCP 危险工具保持关闭。

## License

MIT
