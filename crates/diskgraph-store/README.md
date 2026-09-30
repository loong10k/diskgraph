# diskgraph-store

The persistence layer of [DiskGraph](https://github.com/loong10k/diskgraph):
immutable SQLite snapshots on one side, an authoritative control database on
the other, with versioned migrations between them.

The two databases are deliberately separate. The graph database is
rebuildable by scanning; the control database holds server identity, scope
registrations, policies, jobs and operation history, and must survive the
graph being deleted.

## Install

```toml
[dependencies]
diskgraph-store = "0.2"
```

## Usage

```rust
use diskgraph_store::SqliteSnapshotStore;
use std::path::Path;

let mut store = SqliteSnapshotStore::open(Path::new("/tmp/diskgraph/diskgraph.sqlite"))?;
# Ok::<(), diskgraph_store::StoreError>(())
```

- Migrations run on open: v1 → v2 (revisions, staging, pins) → v3
  (typed entities and relations) → v4 (structured node columns so
  multi-million-row loads never need a full JSON parse per row).
- `open_with_backup` writes a pre-migration copy first, and a failed
  migration leaves the original readable.
- Publication is one transaction: snapshot rows, the revision, the latest
  pointer, and staging cleanup either all land or none do.

## Guarantees

- A snapshot is immutable once published; corrections publish a new one.
- Retention refuses to remove a pinned snapshot, a snapshot backing a
  published revision, or the current latest pointer.
- Deleting graph history never touches the control database.

## License

MIT
