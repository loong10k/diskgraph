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
   a digest over the exact object set. Planning never touches a file.
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

## License

MIT
