# 全平台生产就绪实施记录 — 2026-10-02

**全平台门禁尚未完成。** 规格事实源是 OpenSpec `implement-diskgraph-platform` 的第 15 节及 RE-07；之前的桌面只读门禁不能替代 Android/iOS provider、原生宿主、平台文件操作和发行验收。危险 CLI/MCP 文件工具保持关闭，没有发布或连接生产数据。

本轮已实现并以隔离回归验证：MCP 实际参数 schema、显式 revision/non-root node 和未知参数拒绝；双向关系稳定分页与解码前实体/证据/游标字节预算；失败的内容核验计入尝试次数与真实读取成本；Ops 逐块复核撤权、批准、期限和取消，批准版本绑定原子发布，终态不被覆盖；持久 FFI 服务、真实扫描进度、共享句柄、关闭取消、按 job/fence 固定结果 revision；单项撤权阻止运行和已完成句柄返回缓存数据。Engine 的 scope 列表与 admin fallback 同样复核实时授权。没有持久策略的可信内部兼容入口不用于远程服务。

扫描器 pin/source/digest 保持不变。无法无损恢复的非 Unicode 名称拒绝发布，避免显示名称碰撞选错文件；合法 U+FFFD 名称每个父目录只检查一次。Windows 新增属性句柄观察真实卷序列号和 file ID，128 位 ID 无法无损放入旧 64 位字段时返回 unknown，不截断或猜测。macOS 内容/复制在当前线程关闭 dataless 物化并恢复原策略；这项本机策略测试不代替真实云文件试验，也不代表 Linux/Windows 已有同等保护。

| 平台/能力 | 已取得证据 | 完整门禁缺口 |
| --- | --- | --- |
| macOS arm64 CLI/MCP | 当前 release 二进制 stdio 18/18、认证 HTTP/legacy SSE 13/13；完整 workspace 与窄读基准 | 生产目录长期运行、SLO、备份监控与签名发行 |
| macOS Intel、Linux x64/arm64、Windows x64 | a0ea22a 同 SHA 的 17 项 CI 全绿；1901f89 的五个 Kotlin 和两个 Swift/GRDB 宿主通过 | 1901f89 为 21/22：Windows 原生包并发读取发生数据库锁争用。重复授权写入修复和 schema 9 等待新提交 CI；真实云/写操作和发行验收仍缺 |
| Swift/Kotlin FFI | 两种真实语言宿主扫描/查询/轮询/v1 兼容；Rust FFI 18 项；Swift 动态库与固定 GRDB 的并发 CRUD、描述符释放和重开 | 静态嵌入、Room、GUI 调度、正式 XCFramework/AAR 与应用闭环 |
| 原生文件操作 | macOS 库内隔离回归覆盖撤权、取消、原地修改、目标冲突及保真预算 | Linux/Windows 原生适配、真实卷/占用/权限/恢复与复制保真；公开写入口仍关闭 |
| Android/iOS | URI/provider 模型与明确 unsupported 的能力报告 | provider 实现、授权生命周期、移动包、模拟器与真机验收 |
| 云占位与非 Unicode | macOS 线程策略、本机身份/预算测试；不可逆名称 fail-closed | 真实云 provider 不下载；对应 Linux/Windows 自动化回归已通过；真实云 provider 和卷/设备场景仍缺 |

当前本机只有 `aarch64-apple-darwin` Rust target、Command Line Tools、Android SDK/adb、Swift/Kotlin 命令行宿主；没有 Android NDK、Gradle、完整 Xcode、连接设备或发行签名材料。工具链安装确认尚待用户回复。设备/provider 场景不能用编译、模拟测试或另一平台的成功代替。

本机完整 workspace 和 Clippy `-D warnings` 已通过；两轮独立实现审查没有阻断问题，这是源码与契约证据。[a0ea22a CI](https://github.com/loong10k/diskgraph/actions/runs/36967439579) 为 17/17；[1901f89 CI](https://github.com/loong10k/diskgraph/actions/runs/36973238346) 的五个 Kotlin 和两个 Swift/GRDB 宿主通过，但 Windows 原生包并发门禁失败。已有 principal/permission/scope/policy-version 完整授权键现在只读返回，避免每次本地 CLI 初始化重复写入；持有读事务的回归已复现旧版 database-busy 并验证修复，本机四进程 release 验收通过。该增量的远端验收尚待完成。

较早的 schema 8 release 基准在隔离临时目录中运行，32 字节文件，单独子进程分别测 20k、200k 宽目录与 300 层深目录。[历史原始数据](benchmarks/full-platform-2026-10-02.json)含 p50/p95、扫描和整个进程的峰值 RSS、数据库/WAL，以及四读者并发采样。相同 revision 上完整加载与窄读的配对 top-20 p95 为 12.77→0.23 ms（20k）、108.70→0.38 ms（200k）；正目标候选为 12.69→1.89 ms、102.81→0.44 ms。扫描用时 0.37/4.94 秒，扫描阶段峰值 RSS 36.2/222.6 MB，数据库约 31.4/315.6 MB；全进程峰值 116.5/923.0 MB 包括旧完整加载对照，不能冒充窄读峰值。这些历史值不代表当前 schema 9 的数据库成本或跨平台 SLA；扫描仍无严格 RSS 上限。

Schema 9 在发布事务内持久保存快照节点总数、目录总数/未知数、尺寸降序累计计数。任意 minimum 的精确计数走索引探针，known 页使用对应 partial index；保留旧 JSON 大小 fallback 和未知节点数值字段语义。200k 子项回归证明 SQLite 工作量低于 1,500 VM 步，旧计数为 510 万步，跨越未知行的 known 页为 320 万步。显式 offset 分页仍需 O(offset + page) 跳读。

新增 macOS ARM64 release [聚合原始数据](benchmarks/directory-aggregates-2026-10-02.json)使用不可变合成元数据、各不相同的尺寸和 100 次热缓存计数；不是文件系统扫描基准：

| 子项 | 计数 p95 优化前 → 后 | 备份加迁移 | 数据库迁移前 → 后 | 完整 fenced 发布阶段 |
| --- | --- | --- | --- | --- |
| 20k | 0.6977 → 0.0078 ms | 54.7 ms | 7.08 → 8.59 MB | 194.3 ms |
| 200k | 7.3503 → 0.0070 ms | 611.5 ms | 71.54 → 88.03 MB | 1,901.1 ms |

发布计时包含采样启停、校验、控制事务、图库提交和检查点，不是聚合新增开销或精确持锁时间。迁移备份另占 7.08/71.54 MB；迁移采样 WAL 峰值 2.22/23.66 MB、临时文件 0.68/14.35 MB；发布采样 WAL 峰值 17.31/178.32 MB、临时文件 1.76/9.21 MB。本机每 1 ms 检查文件描述符，包含已 unlink 的 SQLite 临时文件，但采样值不是空间上界。发布前 staging 数据库为 9.84/97.74 MB，发布后为 19.89/203.47 MB。容量规划需要计入备份、staging、WAL 和临时工作，不能只看最终数据库。

关系读取的 200k 条无关边夹具以 SQLite VM 步数验证邻接索引工作量；FFI 深路径分页测试验证响应字节受限且下一 offset 按实际返回条数推进。TUI 递归层共享一个授权读连接、50 ms SQLite 截止时间、整帧 2,048 行（含缓存根层/父节点/翻页探针）、最多四个嵌套页与 256 KiB 展示数据成本。分页不完整时保留父块，不把已读子项重新归一成完整父层；真实读库故障向外传播。先绘入内存缓冲，末段 scope/grant 复核成功后才提交到终端；空闲时每 250 ms 轮询并复核授权，不重复读取地图。这里是查询/展示预算，不承诺严格端到端 UI 延时或峰值 RSS。真实平台设备/发行能力仍未完成。

复验：包定向 `cargo fmt … -- --check`（不要格式化 vendored 源码），`cargo test --workspace --all-targets --locked --no-fail-fast`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`openspec validate implement-diskgraph-platform --strict`；当前二进制运行两个 `scripts/accept-readonly-*.py` 协议脚本；`scripts/ffi-bindings-smoke.sh` 运行两种语言宿主。`--swift-only` 仅提供 Swift 证据；`python3 scripts/accept-ffi-kotlin.py` 默认加载 release 原生库，固定 Maven 插件与 Central 镜像，检查四个节点、Unicode 名称、释放与重开；新增 macOS ARM64/Intel、Linux x64/ARM64、Windows x64 五项真实 JVM 宿主 CI，结果以增量提交为准。`scripts/ffi-grdb-smoke.sh` 同样默认使用 release 原生库，使用 SwiftPM 锁定 GRDB 7.11.1/SHA，实际验证并发 CRUD、原图描述符释放和两库重开。Linux ARM CI/发行均使用原生运行器并执行协议验收。

聚合成本复验：`DISKGRAPH_AGGREGATE_BENCHMARK_OUTPUT="$PWD/docs/benchmarks/directory-aggregates-2026-10-02.json" cargo test --release -p diskgraph-store --lib child_aggregate_benchmark::measure_directory_aggregate_costs -- --ignored --exact --nocapture`。当前 release 二进制的四个并发 CLI 读者由 `scripts/accept-readonly-load.py` 验证。

图库升级为 schema 9、WAL/NORMAL；控制库 FULL。一致性 SQLite 备份后事务创建/回填计数表；快照 writer 标记拒绝升级后仍打开的旧连接写入，计数索引元数据缺失时拒绝查询，不能伪造零计数。断电可丢失最近可重建索引提交，两库没有跨库原子事务。拒绝不明旧归属和不支持的保真条件，不以拒绝代替已实现平台功能。

当前本机全量门禁：568 passed / 13 ignored；schema 9 release 二进制 stdio 18/18、认证 HTTP/SSE 13/13、四进程负载 4/4、Swift/GRDB 与 Kotlin 宿主通过。新增 ignored 聚合基准已单独以 release 执行；其余 12 项真实环境或昂贵夹具不计为通过。最新 20k 文件负载的 32 次 top 查询观察 p50 12.52 ms/p95 17.29 ms，扫描 1.05 秒；这包含 CLI 启动成本，与进程内计数基准口径不同。
