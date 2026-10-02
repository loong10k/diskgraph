# diskgraph-ffi

UniFFI bindings for [DiskGraph](https://github.com/loong10k/diskgraph):
Swift and Kotlin access to the read-only API plus asynchronous job handles.

## Install

The workspace version for host integration is:

```toml
[dependencies]
diskgraph-ffi = "0.3"
```

The generated Swift and Kotlin sources are produced from the built
library:

```bash
cargo build -p diskgraph-ffi
cargo run -p diskgraph-ffi --bin uniffi-bindgen -- generate \
  --library target/debug/libdiskgraph_ffi.dylib --language swift --language kotlin \
  --out-dir bindings
```

## The surface

Read-only JSON envelopes: scan a native path, read the latest snapshot,
list children, explain a node, compare growth, list candidates. Write
operations are not exposed here.

Use a persistent `NativeService` for repeated queries. `spawn_scan` returns
a shared job handle; `poll_result_json` does not wait for scan completion.
Authorization and service startup can access the database, so invoke these
methods from a background thread. Cancellation is cooperative. `shutdown`
refuses new queries and requests cancellation of active session jobs.
`result_json` remains a blocking compatibility API.

Run `scripts/ffi-bindings-smoke.sh` for real Swift/Kotlin hosts and
`scripts/ffi-grdb-smoke.sh` for a pinned GRDB SwiftPM host with concurrent
CRUD alongside the Rust dynamic library, followed by close and reopen.
These tests do not certify static embedding, Android Room, mobile packages
or application UI scheduling. See the [current platform evidence](../../docs/production-readiness-full-platform-2026-10-02.md).

## License

MIT
