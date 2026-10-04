# FFI 绑定验收（P7 任务 9.1 / 9.2 / 9.7；9.3 部分）

## 2026-10-02 持久服务复验

当前 `scripts/ffi-bindings-smoke.sh` 重新构建并生成绑定，Swift 6.4 与 Kotlin 2.4.10/JNA 5.17.0 的真实宿主均通过：持久 NativeService、按实际返回数量推进的字节受限分页、结果轮询、关闭会话、v1 查询，以及 Swift 同进程系统 SQLite CRUD。Kotlin 编译失败现在会使脚本失败，JNA 文件摘要须与固定版本一致。Rust FFI 18 项回归覆盖单项撤权、关闭末段竞态、合并句柄与最后引用释放。

轮询不等待扫描完成，但实时授权需要数据库读取，宿主应从后台线程调用。`result_json` 保留旧阻塞兼容入口；新宿主使用 `poll_result_json`。同根扫描共享句柄与 durable job，所有订阅看到同一 job/fence 发布的 revision。Swift 宿主动态链接 Rust dylib 与系统 SQLite；这不证明静态嵌入或 Android Room。当前 SwiftPM/GRDB 动态链接夹具已完成本机验证，详见下节。

## 2026-10-02 SwiftPM / GRDB 实际共存

`scripts/ffi-grdb-smoke.sh` 默认从当前 release Rust 动态库生成绑定，在临时 SwiftPM 工程中运行固定 GRDB 7.11.1（提交 `b83108d10f42680d78f23fe4d4d80fc88dab3212`，`Package.resolved` + `--force-resolved-versions`）。完整 Xcode 并非该桌面夹具的前置条件，Command Line Tools 已实际构建运行成功。

同一进程的四个 GRDB 写入者与一个读取者执行真实 INSERT/SELECT/UPDATE/DELETE，同时 Rust 扫描及读取 2,001 个节点；最终 400 行、更新值与临时行删除均正确。GRDB 连接池关闭，Rust `shutdown` 拒绝新查询；旧 handle/service 均释放后，Darwin `/dev/fd` + `F_GETPATH` 验证图库、WAL、SHM 描述符归零，再重开两库并验证持久结果。路径使用 `realpath`，避免 `/tmp` 与 `/private/tmp` 别名使释放探针误判。

动态库 SQLite 导出检查与上述行为在本机 macOS ARM64 通过；CI 同时为 macOS ARM64/Intel 执行此脚本，远端结果以对应提交的 [CI](https://github.com/loong10k/diskgraph/actions/workflows/ci.yml) 为准。它不证明静态库、iOS 切片、Room 或 GUI 调度；9.3 保留未完成。

## 2026-10-02 Kotlin/JVM 五平台宿主门禁

`python3 scripts/accept-ffi-kotlin.py` 使用本机 release 动态库，临时 Maven 工程生成并编译 UniFFI Kotlin 绑定；Kotlin 2.4.10、JNA 5.17.0（摘要校验）、resources/compiler/dependency 插件均固定，用户与全局 settings 隔离，所有仓库强制指向 Maven Central。编译不启动驻留 Kotlin daemon。脚本只使用已存在的 Maven/Java，不安装工具。

本机 macOS ARM64 已实际运行成功，断言四个节点、非 ASCII 根路径及 Unicode 子节点、分页/轮询/v1、关闭拒绝、释放后重开与持久数据。[1901f89 CI](https://github.com/loong10k/diskgraph/actions/runs/36973238346) 的 macOS ARM64/Intel、Linux x64/ARM64、Windows x64 五项真实 JVM 宿主，以及两个 Swift/GRDB 宿主均通过；该次完整矩阵为 21/22，Windows 原生包并发查询另有失败，后续增量仍按自身 SHA 验收。

Windows JVM 启动参数夹具以 UTF-8 Base64 ASCII 参数传递非 ASCII 根路径和数据库路径，Kotlin 的可选 `utf8-base64` 标记解码后将真正的 Unicode String 传给 FFI，包括 v1 查询；原两参数模式保留。此修复避免 Windows 启动器参数转换丢失路径，不将 Unicode 文件重命名或替换为 ASCII fixture。桌面 JVM 不代替 Android Room、AAR、GUI 或真机验收。

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
| SQLite 共存（9.3 部分） | 静态库导出计数为 282；实际宿主链接 Rust 动态库与系统 libsqlite3 并调用版本查询，此证据不代表静态嵌入共存 |
| 宿主独立性（9.7） | 空目录 + 无宿主配置下，扫描→查询→能力报告全链路成立；Rust 测试 `the_ffi_layer_is_independent_of_any_host_application` |
| 产物 | Swift 宿主二进制与 Kotlin `host.jar` 均为真实链接产物（非生成源码即验收） |

## 留档的边界（未完成项，不冒充）

- **9.3 完整口径**：SwiftPM/GRDB 的 macOS 动态链接子集已实际验证；静态嵌入、Kotlin/Room、移动端与宿主应用仍未完成。Room 需要对应 Android 构建环境。
- **9.4 / 9.5 / 9.6**（PruneX 界面对接、审阅/批准界面对接、AgentScope 宿主导出边界）：Oct4 只读源码盘点确认工作区中的 PruneX 已接 AgentScope-Swift，但仍使用自身 FileManager／ScanEngine 和 GRDB，尚无 DiskGraph import／调用／包依赖；本仓库也没有真实 AgentScope host tool bridge。库级 ApprovalIssuer 和导出策略测试不能替代 App 审阅签发、GUI 查询结果一致性或云请求截获验收；这些产品链仍需实现并实际构建。
- **9.8**（macOS 原生 App 的扫描/取消/升级/回收恢复闭环）：本验收的宿主是 CLI 形态的原生宿主（覆盖扫描与查询闭环）；带 UI 的取消手势、应用升级迁移、回收/恢复在原生 App 中的闭环留待 PruneX 壳工程。
- **9.9**（XCFramework / Kotlin AAR 打包）：`xcodebuild -create-xcframework` 需要完整 Xcode（本机仅 CLT）；AAR 需要 Android Gradle 构建（属 P9 真机切片范围）。已产出的等价物：`libdiskgraph_ffi.a`（静态库）+ 版本化绑定源 + 两种语言的真实链接运行证据。
