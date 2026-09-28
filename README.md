# DiskGraph

DiskGraph is a read-only disk facts and evidence layer for storage analysis and cleanup products such as PruneX. It turns a directory scan into a queryable snapshot, while keeping cleanup decisions and execution outside the scanner. **The repository is private during initial development; public release is deferred.**

DiskGraph 是面向 PruneX 等清理产品的跨平台磁盘事实层：记录扫描范围、目录节点与可追溯证据，向上提供只读查询。**分类提示不等于删除许可。**

## What works today / 当前进度

- `diskgraph-core`: serializable `DiskSnapshot`, `DiskNode`, and `EvidenceEdge` models; bounded `top`, `children`, `growth`, `explain`, and conservative `candidates` queries.
- `diskgraph-store`: immutable, transactional SQLite snapshots with indexed nodes and evidence; snapshot lookup and paged child queries.
- `diskgraph-disktree`: read-only native-path scanner using [DiskTree](https://github.com/tobi/disktree)'s `disktree-core`, pinned to a reviewed commit. It never invokes DiskTree's removal module.
- `diskgraph-ffi`: UniFFI read-only entry points with generated Swift/Kotlin bindings and a versionable JSON response envelope.
- Automated tests for hidden files, incomplete scans, SQLite persistence, history growth, protected descendants, and JSON round-tripping.

This is an **early foundation**, not a production disk cleaner. Ownership/process collectors, Android URI scanning, iOS document scanning, native-app integration, and cleanup executors are not implemented. Rust CI verifies macOS, Windows, and Linux compilation/tests; this does not establish real-device or product-level support.

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
cargo clippy --workspace --all-targets --locked -- -D warnings
```

On macOS, generate Swift and Kotlin source bindings from the built library:

```bash
cargo build -p diskgraph-ffi --lib --locked
cargo run -p diskgraph-ffi --bin uniffi-bindgen -- generate \
  target/debug/libdiskgraph_ffi.dylib \
  --language swift --language kotlin --no-format --out-dir ./generated-bindings
```

The FFI exports `scan_native_json`, `top_json`, `children_json`, `growth_json`, `explain_json`, and `candidates_json`. Every result is `{"schema_version":1,"ok":true,"data":...}` or `{"schema_version":1,"ok":false,"error":"..."}`. Growth's signed `delta_bytes` is a decimal string to avoid foreign-language integer-width loss. Generated source alone is not an XCFramework, Android AAR, or a tested PruneX integration.

The Rust scanner depends on upstream `disktree-core` at a pinned Git revision. No DiskTree code is vendored into this repository. Both projects use the MIT license.

## License

MIT. See [LICENSE](LICENSE).
