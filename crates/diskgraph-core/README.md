# diskgraph-core

The platform-neutral model behind [DiskGraph](https://github.com/loong10k/diskgraph):
snapshot and evidence types, a bounded query surface, and the scan budgets
and watermarks that keep a walk honest.

Everything here is pure data and pure functions: no IO, no async, no
platform assumptions beyond `OsString` handling in locators.

## Install

```toml
[dependencies]
diskgraph-core = "0.2"
```

## What it gives you

```rust
use diskgraph_core::{DiskNode, NodeKind, ResourceLocator, ScanSettings};

// A locator keeps raw bytes and a display string apart: the display form
// is for logs and UI, never an operation target.
let locator = ResourceLocator::NativePath("/Users/me/project".into());

let node = DiskNode {
    id: 1,
    parent_id: None,
    locator,
    name: "project".into(),
    kind: NodeKind::Directory,
    subtree_bytes: 794_624,
    direct_bytes: 4_096,
    files: 12,
    directories: 3,
    modified_unix_seconds: Some(1_790_000_000),
    file_identity: None,
    category_hint: Some("code".into()),
    reclaim_hint: None,
    read_error: false,
    size_known: true,
};

// Comparable history requires identical scan settings; the store refuses
// to compare snapshots that were taken differently.
let settings = ScanSettings {
    apparent_size: false,
    follow_links: false,
    include_hidden: true,
    one_filesystem: true,
    max_depth: None,
    dedup_hardlinks: true,
};
let _ = (node, settings);
```

Other surfaces, all bounded by construction:

- `children` / `top` — paged, ordered by observed size
- `growth` — refuses comparisons across incompatible snapshots
- `candidates` — a review queue that requires explicit rebuild evidence
- `render_tree` / `render_tree_rows` — a depth-bounded JSON tree for UIs
- `suspect_groups` / `confirm_group` — metadata-only duplicate suspects
- `ScanBudget` — named stops (`NodeLimit`, `StagingLimit`, `TimeLimit`,
  `Cancelled`) rather than silent truncation

## Design notes

- Sizes are **observed** bytes, not guaranteed reclaimable bytes.
- Coverage is a first-class field: a depth-limited or partially unreadable
  walk is reported, never presented as complete.
- Identifiers are typed (`ScopeId`, `RevisionId`, `PrincipalId`) and
  serialized as strings, so a display value is never mistaken for an
  authorization token.

## License

MIT
