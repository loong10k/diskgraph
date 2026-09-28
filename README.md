# DiskGraph

DiskGraph is an open-source, read-only disk facts and evidence layer for storage analysis and cleanup products such as PruneX. It turns a directory scan into a queryable snapshot, while keeping cleanup decisions and execution outside the scanner.

DiskGraph 是面向 PruneX 等清理产品的跨平台磁盘事实层：记录扫描范围、目录节点与可追溯证据，向上提供只读查询。**分类提示不等于删除许可。**

## What works today / 当前进度

- `diskgraph-core`: serializable `DiskSnapshot`, `DiskNode`, and `EvidenceEdge` models; bounded `top`, `children`, `growth`, `explain`, and conservative `candidates` queries.
- `diskgraph-disktree`: read-only native-path scanner using [DiskTree](https://github.com/tobi/disktree)'s `disktree-core`, pinned to a reviewed commit. It never invokes DiskTree's removal module.
- Automated tests for hidden files, incomplete scans, growth comparison, protected descendants, and JSON round-tripping.

This is an **early foundation**, not a production disk cleaner. Snapshots are currently in memory; SQLite persistence, native bindings, ownership/process evidence, and cleanup executors are not implemented. The native-path adapter is currently tested on macOS only. Android, iOS, Windows, and Linux support are design targets, not verified releases.

## Architecture / 架构

```text
PruneX UX + explicit approval + platform cleanup executor
                         |
                DiskGraph read-only API
           snapshot + nodes + evidence
                         |
           platform discovery adapters
              DiskTree native paths
          Android/iOS documents (planned)
```

`ResourceLocator` is an observation locator, not a capability or deletion handle. Query results are for human review; an executor must independently revalidate the exact resource, permissions, liveness, and user approval before any mutation. Scan sizes are not guaranteed reclaimable bytes; actual space changes require before/after volume measurements.

See [architecture and roadmap](docs/architecture.md) for platform boundaries and next steps.

## Build / 构建

Requires Rust 1.97 or newer:

```bash
cargo test --workspace --locked
cargo fmt --all --check
```

The Rust scanner depends on upstream `disktree-core` at a pinned Git revision. No DiskTree code is vendored into this repository. Both projects use the MIT license.

## License

MIT. See [LICENSE](LICENSE).
