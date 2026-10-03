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

源码按职责拆为 UniFFI 函数导出、数据库 realm、扫描协调、作业句柄与共享状态。
`lib.rs` 保留模块声明、明确导出和必需 UniFFI scaffolding。两份真实 API/JobHandle
实现通过固定 `include!` 保持旧词法根及绑定 metadata 校验值；源码门禁解析实际包含文件，
不允许其他包含路径或重复挂载。运行期 metadata 文档保持兼容，中文契约通过
`cfg_attr(doc, doc = ...)` 显示在 Rustdoc 中；原有函数和对象仍从 crate 根公开。JSON envelope、作业取消/授权、会话锁与 Engine 所有权保持原义。
内部结果、授权闭包和任务弱引用列表各自独立文件，不新增运行时包装层。
生成器源文件使用 `bin/uniffi_bindgen.rs`，Cargo 显式维持 `uniffi-bindgen` 命令名。
