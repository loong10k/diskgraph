# 全平台生产就绪实施记录 — 2026-10-02

**全平台门禁尚未完成。** 规格事实源是 OpenSpec `implement-diskgraph-platform` 的第 15 节及 RE-07；之前的桌面只读门禁不能替代 Android/iOS provider、原生宿主、平台文件操作和发行验收。危险 CLI/MCP 文件工具保持关闭，没有发布或连接生产数据。

本轮已实现并以隔离回归验证：MCP 实际参数 schema、显式 revision/non-root node 和未知参数拒绝；双向关系稳定分页与解码前实体/证据/游标字节预算；失败的内容核验计入尝试次数与真实读取成本；Ops 逐块复核撤权、批准、期限和取消，批准版本绑定原子发布，终态不被覆盖；持久 FFI 服务、真实扫描进度、共享句柄、关闭取消、按 job/fence 固定结果 revision；单项撤权阻止运行和已完成句柄返回缓存数据。Engine 的 scope 列表与 admin fallback 同样复核实时授权。没有持久策略的可信内部兼容入口不用于远程服务。

扫描器 pin/source/digest 保持不变。无法无损恢复的非 Unicode 名称拒绝发布，避免显示名称碰撞选错文件；合法 U+FFFD 名称每个父目录只检查一次。Windows 新增属性句柄观察真实卷序列号和 file ID，128 位 ID 无法无损放入旧 64 位字段时返回 unknown，不截断或猜测。macOS 内容/复制在当前线程关闭 dataless 物化并恢复原策略；这项本机策略测试不代替真实云文件试验，也不代表 Linux/Windows 已有同等保护。

| 平台/能力 | 已取得证据 | 完整门禁缺口 |
| --- | --- | --- |
| macOS arm64 CLI/MCP | 当前 release 二进制 stdio 18/18、认证 HTTP/legacy SSE 13/13；完整 workspace 与窄读基准 | 生产目录长期运行、SLO、备份监控与签名发行 |
| macOS Intel、Linux x64/arm64、Windows x64 | 历史只读原生门禁；本轮增加 Linux ARM 原生 CI 与 Windows 身份回归 | 本轮同 SHA 的远端结果须单独记录，不能把工作流配置称为通过 |
| Swift/Kotlin FFI | 两种真实语言宿主扫描/查询/轮询/v1 兼容；Rust FFI 18 项；Swift 同进程系统 SQLite CRUD | GRDB/Room 共存、GUI 调度、正式 XCFramework/AAR 与应用闭环 |
| 原生文件操作 | macOS 库内隔离回归覆盖撤权、取消、原地修改、目标冲突及保真预算 | Linux/Windows 原生适配、真实卷/占用/权限/恢复与复制保真；公开写入口仍关闭 |
| Android/iOS | URI/provider 模型与明确 unsupported 的能力报告 | provider 实现、授权生命周期、移动包、模拟器与真机验收 |
| 云占位与非 Unicode | macOS 线程策略、本机身份/预算测试；不可逆名称 fail-closed | 真实云 provider 不下载；Linux 非 Unicode 碰撞与 Windows 原生句柄须在对应系统验证 |

当前本机只有 `aarch64-apple-darwin` Rust target、Command Line Tools、Android SDK/adb、Swift/Kotlin 命令行宿主；没有 Android NDK、Gradle、完整 Xcode、连接设备或发行签名材料。工具链安装确认尚待用户回复。设备/provider 场景不能用编译、模拟测试或另一平台的成功代替。

本机完整 workspace、Clippy `-D warnings`、包定向 fmt 和 OpenSpec strict 已执行；scope 列表的新增撤权回归及受影响 MCP/FFI 目标再次通过。独立代码审查为 APPROVE，架构复核为 CLEAR；两者是源码与契约证据。最终同 SHA 全量及远端证据将在门禁结果出齐后补记。

Release 基准在隔离临时目录中运行，32 字节文件，单独子进程分别测 20k、200k 宽目录与 300 层深目录。[原始数据](benchmarks/full-platform-2026-10-02.json)含 p50/p95、扫描和整个进程的峰值 RSS、数据库/WAL，以及四读者并发采样。相同 revision 上完整加载与窄读的配对 top-20 p95 为 12.77→0.23 ms（20k）、108.70→0.38 ms（200k）；正目标候选为 12.69→1.89 ms、102.81→0.44 ms。扫描用时 0.37/4.94 秒，扫描阶段峰值 RSS 36.2/222.6 MB，数据库约 31.4/315.6 MB；全进程峰值 116.5/923.0 MB 包括旧完整加载对照，不能冒充窄读峰值。这些是热缓存观察，非跨平台比较或 SLA；扫描仍无严格 RSS 上限。

关系读取的 200k 条无关边夹具以 SQLite VM 步数验证邻接索引工作量；FFI 深路径分页测试验证响应字节受限且下一 offset 按实际返回条数推进。宽目录精确聚合、TUI 每帧整体工作预算，以及真实平台设备/发行能力仍是未完成项。

复验：包定向 `cargo fmt … -- --check`（不要格式化 vendored 源码），`cargo test --workspace --all-targets --locked --no-fail-fast`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`openspec validate implement-diskgraph-platform --strict`；当前二进制运行两个 `scripts/accept-readonly-*.py` 协议脚本；`scripts/ffi-bindings-smoke.sh` 运行两种语言宿主。`--swift-only` 仅提供 Swift 证据。Linux ARM CI/发行均使用原生运行器并执行协议验收。

图库继续 schema 8、WAL/NORMAL；控制库 FULL。断电可丢失最近可重建索引提交，两库没有跨库原子事务。迁移前一致性备份与隔离升级/回滚门禁继续保留，拒绝不明旧归属和不支持的保真条件，不以拒绝代替已实现平台功能。

最终本机全量门禁：556 passed / 12 ignored；当前二进制 stdio 18/18、认证 HTTP/SSE 13/13；Swift/Kotlin 两宿主通过。12 项 ignored 需要真实环境或昂贵夹具，不计为通过；release 隔离基准已单独执行。
