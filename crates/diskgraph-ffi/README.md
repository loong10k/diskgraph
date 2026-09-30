# diskgraph-ffi

UniFFI bindings for [DiskGraph](https://github.com/loong10k/diskgraph):
Swift and Kotlin access to the read-only API plus asynchronous job handles.

## Install

The Rust crate is published for host integration:

```toml
[dependencies]
diskgraph-ffi = "0.2"
```

The generated Swift and Kotlin sources are produced from the built
library:

```bash
cargo build -p diskgraph-ffi
cargo run -p diskgraph-ffi --bin uniffi-bindgen -- generate \
  target/debug/libdiskgraph_ffi.dylib --language swift --language kotlin \
  --out-dir bindings
```

## The surface

Read-only JSON envelopes: scan a native path, read the latest snapshot,
list children, explain a node, compare growth, list candidates. Write
operations are not exposed here.

`spawn_scan_json` returns a handle immediately — a UI thread never blocks
on a walk. `progress_json` is a non-blocking snapshot, `cancel` is
cooperative, and `result_json` is an idempotent join whose error path
reports a caller cancellation honestly.

## License

MIT
