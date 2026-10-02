# FFI 绑定验收（P7 任务 9.1 / 9.2 / 9.7；9.3 部分）

## 2026-10-02 持久服务复验

当前 `scripts/ffi-bindings-smoke.sh` 重新构建并生成绑定，Swift 6.4 与 Kotlin 2.4.10/JNA 5.17.0 的真实宿主均通过：持久 NativeService、按实际返回数量推进的字节受限分页、结果轮询、关闭会话、v1 查询，以及 Swift 同进程系统 SQLite CRUD。Kotlin 编译失败现在会使脚本失败，JNA 文件摘要须与固定版本一致。Rust FFI 18 项回归覆盖单项撤权、关闭末段竞态、合并句柄与最后引用释放。

轮询不等待扫描完成，但实时授权需要数据库读取，宿主应从后台线程调用。`result_json` 保留旧阻塞兼容入口；新宿主使用 `poll_result_json`。同根扫描共享句柄与 durable job，所有订阅看到同一 job/fence 发布的 revision。Swift 宿主动态链接 Rust dylib 与系统 SQLite；这不证明静态库与 GRDB/Room 的工程级符号隔离，后者仍未完成。

以下 2026-09-29 记录是旧版历史证据，旧“立即/UI 不阻塞”和系统 SQLite 版本查询不代表完整生产宿主验收。当前平台缺口见[全平台实施记录](../production-readiness-full-platform-2026-10-02.zh-CN.md)。

日期：2026-09-29 · 主机：macOS（darwin 27.0.0，arm64，Apple Silicon，Swift 6.4 CLI / Kotlin 2.x + JNA 5.17 / OpenJDK 21）· 执行：`scripts/ffi-bindings-smoke.sh`

结果：**全部通过**。脚本从构建产物重新生成版本化绑定，然后用两种真实语言宿主编译并运行。

## 实测内容

| 检查 | 结果 |
| --- | --- |
| 绑定生成（9.1） | `uniffi-bindgen 0.32.2` 从 `libdiskgraph_ffi.dylib` 生成 Swift（`.swift` + modulemap + 头）与 Kotlin（`.kt`）到 `dist/ffi/`，含 `JobHandle`/`spawnScanJson` 新面 |
| v1 兼容（9.1，PF-01） | v1 只读面（`scanNativeJson`/`topJson`/`latestNativeSnapshotJson`）在两个宿主中照常工作；Rust 侧 v1 测试持续通过 |
| 值往返（9.1） | 整数（`node_count`）、错误 envelope（`ok:false` + exit 码）、未知值（`null` progress result）在 JSON 边界往返一致 |
| 异步句柄（9.2，PF-01） | `spawnScanJson` **立即**返回（宿主打印 `state:"running"`），`progressJson` 非阻塞，`resultJson` 幂等 join；取消是诚实的——要么完整成功要么报告取消，无半发布产物（Rust 测试 `a_cancelled_job_reports_honestly_instead_of_half_publishing`） |
| SQLite 共存（9.3 部分） | 静态库导出 282 个 `sqlite3_` 符号，宿主同进程链接系统 libsqlite3（3.54.0）并调用 `sqlite3_libversion()`——两套 SQLite 共存编译、链接、运行 |
| 宿主独立性（9.7） | 空目录 + 无宿主配置下，扫描→查询→能力报告全链路成立；Rust 测试 `the_ffi_layer_is_independent_of_any_host_application` |
| 产物 | Swift 宿主二进制与 Kotlin `host.jar` 均为真实链接产物（非生成源码即验收） |

## 留档的边界（未完成项，不冒充）

- **9.3 完整口径**（Swift/GRDB 与 Kotlin/Room 共存夹具）：GRDB 需要 Xcode/SwiftPM 工程、Room 需要 Gradle/Android SDK——本机仅有 Command Line Tools（无 `xcodebuild`）。本验收以"系统 SQLite 与静态库同进程共存运行"作为符号/链接层证据；GRDB/Room 工程级夹具留待装齐工具链的主机。
- **9.4 / 9.5 / 9.6**（PruneX 界面对接、审阅/批准界面对接、AgentScope 宿主导出边界）：对接的另一侧（PruneX / AgentScope-Swift/Kotlin）不在本仓库，无法单侧完成；FFI 层的对应能力面（审批需可信签发、导出策略元数据优先）已在 ops/engine 层实现并有测试。
- **9.8**（macOS 原生 App 的扫描/取消/升级/回收恢复闭环）：本验收的宿主是 CLI 形态的原生宿主（覆盖扫描与查询闭环）；带 UI 的取消手势、应用升级迁移、回收/恢复在原生 App 中的闭环留待 PruneX 壳工程。
- **9.9**（XCFramework / Kotlin AAR 打包）：`xcodebuild -create-xcframework` 需要完整 Xcode（本机仅 CLT）；AAR 需要 Android Gradle 构建（属 P9 真机切片范围）。已产出的等价物：`libdiskgraph_ffi.a`（静态库）+ 版本化绑定源 + 两种语言的真实链接运行证据。
