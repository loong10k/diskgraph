# 全平台生产就绪实施记录 — 2026-10-02

**全平台门禁尚未完成。** 规格事实源是 OpenSpec `implement-diskgraph-platform` 的第 15 节及 RE-07；之前的桌面只读门禁不能替代 Android/iOS provider、原生宿主、平台文件操作和发行验收。危险 CLI/MCP 文件工具保持关闭，没有发布或连接生产数据。

本轮已实现并以隔离回归验证：MCP 实际参数 schema、显式 revision/non-root node 和未知参数拒绝；双向关系稳定分页与解码前实体/证据/游标字节预算；失败的内容核验计入尝试次数与真实读取成本；Ops 逐块复核撤权、批准、期限和取消，批准版本绑定原子发布，终态不被覆盖；持久 FFI 服务、真实扫描进度、共享句柄、关闭取消、按 job/fence 固定结果 revision；单项撤权阻止运行和已完成句柄返回缓存数据。Engine 的 scope 列表与 admin fallback 同样复核实时授权。没有持久策略的可信内部兼容入口不用于远程服务。

扫描器 pin/source/digest 保持不变。无法无损恢复的非 Unicode 名称拒绝发布，避免显示名称碰撞选错文件；合法 U+FFFD 名称每个父目录只检查一次。Windows 属性句柄观察真实卷序列号和 file ID，128 位 ID 无法无损放入旧快照 64 位字段时返回 unknown，不截断或猜测；内容检查另在私有句柄状态保留完整原生 ID。macOS 内容/复制在当前线程关闭 dataless 物化并恢复原策略；本机策略测试及 Windows 线程暴露/NTFS 夹具均不代替真实云 provider 不下载验收，Linux 保护仍未完成。

| 平台/能力 | 已取得证据 | 完整门禁缺口 |
| --- | --- | --- |
| macOS arm64 CLI/MCP | 当前 release 二进制 stdio 18/18、认证 HTTP/legacy SSE 13/13；完整 workspace 与窄读基准 | 生产目录长期运行、SLO、备份监控与签名发行 |
| macOS Intel、Linux x64/arm64、Windows x64 | 原生内容代码提交 ae0223c 的 22/22 项 CI 通过，含八个 Rust、五个 Kotlin、两个 Swift/GRDB 宿主和五个原生包 | 真实云/写操作、发行和生产长期运行验收仍缺 |
| Swift/Kotlin FFI | 两种真实语言宿主扫描/查询/轮询/v1 兼容；Rust FFI 18 项；Swift 动态库与固定 GRDB 的并发 CRUD、描述符释放和重开 | 静态嵌入、Room、GUI 调度、正式 XCFramework/AAR 与应用闭环 |
| 原生文件操作 | macOS 库内隔离回归覆盖撤权、取消、原地修改、目标冲突及保真预算 | Linux/Windows 原生适配、真实卷/占用/权限/恢复与复制保真；公开写入口仍关闭 |
| Android/iOS | URI/provider 模型与明确 unsupported 的能力报告 | provider 实现、授权生命周期、移动包、模拟器与真机验收 |
| 云占位与非 Unicode | macOS 线程策略、Windows 线程暴露及本地 NTFS offline/reparse 拒绝；不可逆名称 fail-closed | 真实 provider 不下载验收、Linux 保护及其他卷/设备场景仍缺 |

当前本机只有 `aarch64-apple-darwin` Rust target、Command Line Tools、Android SDK/adb、Swift/Kotlin 命令行宿主；没有 Android NDK、Gradle、完整 Xcode、连接设备或发行签名材料。工具链安装确认尚待用户回复。设备/provider 场景不能用编译、模拟测试或另一平台的成功代替。

历史连接接纳增量的本机完整 workspace 为 584 passed / 13 ignored，Clippy `-D warnings` 通过。独立审查发现并复现了 runner 停止后的认领、无常驻服务的过期任务恢复和取消终态三个边界；针对性修复已先红后绿。[历史 4edfac0 CI](https://github.com/loong10k/diskgraph/actions/runs/36976148057) 为 20/22，Windows 四进程查询报 SQLite I/O，macOS ARM 的扫描预算拒绝偶尔返回成功。后续 [f57aa40 CI](https://github.com/loong10k/diskgraph/actions/runs/36980741836) 已 22/22 全绿，包含两个受影响的原生包。这证明该增量门禁通过，尚不能锁定唯一 Windows VFS 根因或代替生产长期运行证据。

单次 CLI 查询现不启动后台队列，`--wait` 核验实际任务终态并按该任务/fence 取得 revision；只条件接管指定的过期任务，或终结该任务的过期取消/撤权 owner，不修改无关任务、不抢占存活租约。等待期限仅覆盖等待和开始接管，扫描有独立 Engine 预算。runner Drop 在队列读前、返回后及每次认领前检查停止；已通过检查的工作继续受原预算/租约约束，不承诺瞬时取消。SQLite 错误保留扩展码，启动失败带阶段标记。该行为增量已通过上述 f57aa40 跨平台 CI。

较早的 schema 8 release 基准在隔离临时目录中运行，32 字节文件，单独子进程分别测 20k、200k 宽目录与 300 层深目录。[历史原始数据](benchmarks/full-platform-2026-10-02.json)含 p50/p95、扫描和整个进程的峰值 RSS、数据库/WAL，以及四读者并发采样。相同 revision 上完整加载与窄读的配对 top-20 p95 为 12.77→0.23 ms（20k）、108.70→0.38 ms（200k）；正目标候选为 12.69→1.89 ms、102.81→0.44 ms。扫描用时 0.37/4.94 秒，扫描阶段峰值 RSS 36.2/222.6 MB，数据库约 31.4/315.6 MB；全进程峰值 116.5/923.0 MB 包括旧完整加载对照，不能冒充窄读峰值。这些历史值不代表当前 schema 9 的数据库成本或跨平台 SLA；扫描仍无严格 RSS 上限。

Schema 9 在发布事务内持久保存快照节点总数、目录总数/未知数、尺寸降序累计计数。任意 minimum 的精确计数走索引探针，known 页使用对应 partial index；保留旧 JSON 大小 fallback 和未知节点数值字段语义。200k 子项回归证明 SQLite 工作量低于 1,500 VM 步，旧计数为 510 万步，跨越未知行的 known 页为 320 万步。显式 offset 分页仍需 O(offset + page) 跳读。 MCP children 新 v2 游标绑定真实主体、scope/revision、父节点、minimum、排序和当前策略 epoch，按尺寸降序/name/id 升序 seek；保留显式 offset/next_offset。200k 的同尺寸及不同尺寸深页均低于 1,500 VM 步。搜索续页改为 name/id 索引 seek，200k 全匹配深页由约 160 万步降到低于 2,000 步，保留 Rust Unicode 小写子串语义。页外 lookahead 仅检查存在，不转换 Rust 节点/定位字符串；字节预算截页后游标从实际末项继续。children 的 64 KiB 预算针对工具数据，JSON-RPC/text/structuredContent 包装另有成本。

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

此前 4edfac0 本机全量门禁：568 passed / 13 ignored；当时 schema 9 release 二进制 stdio 18/18、认证 HTTP/SSE 13/13、四进程负载 4/4、Swift/GRDB 与 Kotlin 宿主通过。新增 ignored 聚合基准已单独以 release 执行；其余 12 项真实环境或昂贵夹具不计为通过。最新 20k 文件负载的 32 次 top 查询观察 p50 12.52 ms/p95 17.29 ms，扫描 1.05 秒；这包含 CLI 启动成本，与进程内计数基准口径不同。

f57aa40 的 release 二进制通过 stdio 18/18、HTTP/SSE 13/13 与四进程负载 4/4；[原始负载数据](benchmarks/keyset-readonly-load-2026-10-02.json)记录 20k 文件扫描 0.406 秒、32 次 top/children 查询 p50 6.272 ms/p95 9.534 ms、数据库与 WAL 合计 37,445,632 字节。不同运行存在缓存/调度差异，单凭这组测量不能定位 Windows I/O 原因或建立生产 SLA。

## 存储结构增量（ST-06 / 15.7）

存储入口从 3,247 行降为 93 行，只保留模块声明和稳定导出。真实查询、发布、迁移、控制及操作持久化按职责归入模块；最大的生产模块为 410 行的任务存储。每个类型有独立文件，中文注释说明实际原生 Rust 来源和参数/返回语义，未虚构 Java 对应、未增加公开仓储包装或连接 owner。[存储 README](../crates/diskgraph-store/README.md#source-boundaries--源码边界)及两种语言的架构文档展示了这些边界。

新增 AST 源码结构回归先在旧多类型入口和 wildcard import 上失败，再于拆分后通过；同时检查中文注释、拒绝占位注释和生产 stub。原有测试保留：store 全目标 76 passed / 5 ignored，workspace 582 passed / 13 ignored；完整 Clippy、定向 fmt、OpenSpec strict 和 release CLI/MCP/FFI 构建通过。release stdio 18/18、HTTP/SSE 13/13、四进程负载 4/4 再次通过。[本次隔离负载数据](benchmarks/store-structure-load-2026-10-02.json)为 20k 扫描 0.423 秒、32 次查询 p50 7.389 ms/p95 11.24 ms、数据库/WAL 37,445,632 字节。本次结构整理不宣称带来测得的提速。

独立代码及架构复核确认公开导出、tuple 列顺序、API/wire 字段、事务/fencing/预算行为及 185 处 SQL 字符串保持一致。人工审查发现并修正了新增中文注释中历史管理授权、可信状态 setter、连接/事务责任、任务配额返回和关系整页预算的错误描述；源码门禁不能代替这种语义审查。两位审查者已批准本增量；[同一代码 SHA 63bc6c7 的 22/22 项 CI 已全绿](https://github.com/loong10k/diskgraph/actions/runs/36986111434)，包含五个原生包和两种真实语言宿主矩阵。ST-06 仅覆盖存储 crate，不代表其他 Rust crate 全部符合组织规范，也不代替尚缺的全平台能力验收。

## 连接接纳后续修复（MCP-06 / 15.8）

[后续纯文档提交 1ba56aa 的 CI](https://github.com/loong10k/diskgraph/actions/runs/36987130326) 为 21/22，Windows 连接上限测试暴露 TCP reset。它没有改变存储实现；此前 22/22 是一次成功验收，不能证明这条拒绝边界稳定。将旧测试扩展为读取完整 503 正文后，在 macOS 也先复现了同样 reset。

超上限连接现发送 503 Service Unavailable 与 Connection: close，再半关闭写端，以固定 4 KiB 缓冲、64 KiB 累计字节预算和发送前固定的 50 ms 绝对期限清理输入。不新增工作线程，不解码请求、不调用 Engine；原公开响应写入接口仍保持 keep-alive。超量或静默拒绝输入可在预算处终止，OS 调度不属于硬墙钟上限。同步清理会短暂暂停新接纳；这是资源有界保证，不承诺洪流公平性或拒绝吞吐 SLA。

完整正文/Content-Length/关闭响应回归与两个真实 socket 的静默/超量预算测试通过。完整 workspace 584 passed / 13 ignored，Clippy、定向 fmt、OpenSpec strict 与 release 二进制通过，stdio 18/18、HTTP/SSE 13/13。两位独立复审者批准；审查者额外滴流探针中，首连接每 20 ms 发送字节，第二连接于 51.373 ms 收到拒绝；这是一次观察值，不是 SLA。源码实现阶段仍待新 SHA 的 Windows 原生 CI；本机回归和旧 SHA 的全绿不能代替它。

后续源码提交 23cfb52 的[同 SHA 22/22 项 CI 全绿](https://github.com/loong10k/diskgraph/actions/runs/36989443313)。Windows stable 和 Rust 1.97.0 均通过完整响应、静默客户端和 64 KiB 清理回归；Windows stable MCP 日志实际为 99 passed / 0 failed。五个原生包、八个 Rust、五个 Kotlin 宿主和两个 Swift/GRDB 宿主全部通过；15.8 按这条有界拒绝行为关闭，不代替移动 provider、原生写操作、签名或生产长期运行验收。

## 客户端限流与代理边界（MCP-06 / 15.9）

本轮先复现五项失败：新客户端可无限扩张限流表、超长键被保留、较早时钟观测重复补充额度、IPv6 被冒号截断，以及可信代理链选择攻击者伪造的左端前缀。限流实现现以单调时钟和共享时间水位补充，最多 4096 桶、64 字节键；满表不逐出活跃桶，只允许 60 秒闲置桶回收，扫描最多每秒一次。已存在的键借用查找，避免逐请求分配同一个键。状态按 IP、每个服务实例独立，NAT 共用额度，重启清空；满表时包括有权限主体在内的新 IP 也得到 429，活跃桶可持续占位，`retry_after_ms=1000` 不保证一秒后恢复容量。这里只保证状态与扫描成本有界，不承诺精确 RSS 字节或洪流公平性。

网络输入使用规范化 IPv4/IPv6（映射地址统一），可信代理用精确 IP 匹配，最多解析 32 个合法 hop，从右向左跨越可信代理并停在第一个不可信 hop。代理须覆盖不可信输入或追加实际 peer；非法/过长链回退到直接 peer，多用户可能共用代理额度。授权仍由 token 与实时 grant 决定。两位独立审查者进一步发现 HTTP parser 只保留第一条重复 XFF，会丢失代理追加字段；新的真实 socket 夹具也先红，再修复为按接收顺序合并，原总头预算不变。列表字段合并顺序依据 [RFC 9110 §5.3](https://www.rfc-editor.org/rfc/rfc9110.html#name-field-order)。

旧 `http::RateLimiter` 与 `http::observed_client_ip` 公开路径保留，新增实现每类型独立文件并使用中文原生来源说明。六项新回归先红后绿，完整 workspace 为 590 passed / 13 ignored；Clippy `-D warnings`、定向 fmt、OpenSpec strict、上游 pin/一致性回归通过，vendor 源码与摘要未变。release CLI/MCP/FFI 构建、stdio 18/18 与 HTTP/SSE 13/13 通过。独立代码与架构复审已批准并关闭重复 XFF 阻断；代码审查者独立重跑六项回归、两项 framing/header-budget 及外部重复头 socket 探针。[源码提交 9fefc78 的同 SHA 22/22 原生 CI](https://github.com/loong10k/diskgraph/actions/runs/36997233647)全部通过；Windows stable 与 1.97.0 日志逐项记录六项新回归通过、MCP 105 passed。15.9 按此限定行为完成，全平台 15.2–15.6 的其余能力仍未完成。

后续隔离认证 socket 探针发现另一独立缺口：`max_response_bytes=128` 时 legacy tools/list 仍投递 7511 字节，原分发绕过现代 HTTP 结果字节门禁，待发送队列只有 64 条计数。临时数据库探针复现了断言失败，并不证明发生实际 OOM。修复及验收记录如下，不从旧阶段协议成功推导该边界已通过。

## Legacy 投递与实时授权（MCP-05/06 / 15.10）

远程 legacy 投递现使用私有预留注册表。在应答或执行工具之前，按配置的最坏响应加 22 字节 SSE 包装预留，每会话最多 64 条/16 MiB、每监听器共 64 MiB，覆盖执行、编码、排队和分块写入。容量耗尽在副作用前返回 HTTP 429，单次预留无法容纳时返回 413；默认最大响应下每会话最多三个、监听器最多十五个最坏预留。编码后缩减为实际 UTF-8 wire 字节，队列/在途/已关闭会话的预留在完成或 Drop 时退款；断线后仍在执行的工作保留占额直到退出。远程路径不暴露裸 sender；旧 `SessionRegistry` 只保留为可信内部兼容 API。

共用 JSON writer 在序列化时拒绝超量输出。Legacy 超限返回保留原请求 ID 的完整 `response_too_large` JSON-RPC 错误；连这一关联错误也无法容纳时，在执行前返回 HTTP 413。同一认证探针现通过：**128 字节限制下，结果从 7511 降为 79 payload 字节**。现代 HTTP 共用有界 writer，原错误状态及 wire 字段保留。预算不覆盖工具已构造的 `serde_json::Value`、分配器开销、内核缓冲，也不构成严格 RSS 上限；固定小型传输诊断另有明确最小开销。

控制库 v6 用事务触发器维护 policy/grant/scope 的独立授权 generation，不依赖 policy epoch，任务/操作更新不使流失效。结果记录其 generation；分块写入之间检查 token 到期及窄读 generation，任何授权变更都会保守关闭旧结果流，包括无关 grant 的变化。共享 Engine 锁采用非阻塞尝试，SQLite 执行和锁等待使用剩余绝对写入期限，实际 socket 写入前再次检查期限。helper 还原真实原 busy_timeout，并清除自身执行回调。独立审查先复现旧 bug：100 ms 期限遇到 300 ms 控制锁仍发送私有字节；修复后探针于 100.2645 ms 超时且发送零字节。这是一次观测，不是端到端 SLA；外层完整授权检查仍可能等待策略访问，撤权也无法收回内核已接受的字节。

v5→v6 升级先做一致性 pre-v6 备份，再原子迁移。失败注入回归确认回滚、原策略数据保留及备份存在；跨连接授权变更、保留另一 scope 权限时的单项撤销、重复/回滚变更和 generation 溢出均有回归。升级前须停止旧服务/宿主，迁移不会修复正在运行的旧传输代码，不承诺混合版本滚动安全。Store 的 `lib.rs` 当前为 94 行，仍只声明和重导出；授权计数逻辑位于独立文件。

本机 workspace 为 **614 passed / 0 failed / 13 ignored**；Clippy `-D warnings`、定向 fmt、OpenSpec strict 与 release CLI/MCP/FFI 构建通过。release stdio 18/18、认证 HTTP/SSE 13/13、隔离四进程负载 4/4。[新增原始负载数据](benchmarks/legacy-delivery-load-2026-10-02.json)为 20k 文件扫描 0.541 秒、32 次 top/children 查询 p50 11.973 ms/p95 16.864 ms、数据库/WAL 37,449,728 字节；本次不据此宣称提速或生产 SLO。独立代码和架构复审批准本增量，包含外部序列化及锁/撤权探针。[源码提交 2bef5e5 的同 SHA 22/22 CI](https://github.com/loong10k/diskgraph/actions/runs/37005829815)全绿；Windows stable 和 1.97.0 日志逐项确认 21 项新增单元回归（传输七项、预留六项、授权计数六项、编码两项）、三项认证 legacy socket 回归及 MCP 单元 120 passed。五个原生包的升级/备份回滚和协议门禁也通过。15.10 按此有界行为完成，15.2–15.6 的其余能力保持未完成。

## Windows 普通文件内容句柄（CT-01/02/05 / 15.11）

Windows 本地普通文件的 `read_bounded` 与摘要检查已接入原生句柄，替代此前一律 unsupported 的路径。路径计划拒绝父目录穿越、ADS、UNC/设备命名空间、超长路径及超多组件；要求注册根的精确组件拼写，不猜测大小写或 8.3 别名等价。逐组件相对保留的父目录句柄打开已有对象，在申请文件数据前检查原生类型、卷与 reparse/占位属性。私有状态保留完整 128 位 file ID、卷、长度、创建/写入/change 时间和待删除状态；未扩大旧快照的 64 位身份及秒级时间契约。

当前线程通过动态解析的可选 Windows API 暴露占位属性，RAII 恢复原模式；能力缺失映射公开 `unsupported`。属性获取阶段不能冻结新 writer，数据打开前观察到变化返回 `Conflict`。数据句柄限制普通写入/删除共享；活动 writer 冲突，检查期间父目录重命名被拒绝，读取中及结束后复核原生状态。这些约束不能排除所有可写映射、内核或过滤器修改，观察稳定不等于原子内容快照；原生时间单位也不保证文件系统精度。`FILE_OPEN_NO_RECALL` 约束打开过程，不能保证所有 provider 的后续读取；offline 属性夹具不是真实 provider 不下载试验。打开/读取仍同步，期限与取消检查采用协作方式。

真实 Windows RED 来自[测试先行的 4b307a9](https://github.com/loong10k/diskgraph/actions/runs/37012219071)：编译成功后，线程模式回归及最初十一项内容测试中的八项失败。[第一版实现](https://github.com/loong10k/diskgraph/actions/runs/37013605248)通过十项内容测试，暴露了属性阶段 writer 假设错误；随后修正契约与测试，要求获取时修改产生冲突，并另加确定性测试验证数据读取/摘要期间 writer 受限。[下一次原生运行](https://github.com/loong10k/diskgraph/actions/runs/37015675642)十二项内容及九项内容比较全部通过，但公共预算夹具使用了 TEMP 根的 8.3 别名。仅对可信夹具根 canonicalize，未放宽生产范围边界。

独立审查还复现了跨平台摘要缺陷：100 ms 期限在控制锁等待 300 ms 后仍读取字节，或确认空文件摘要。现于阻塞授权之后、下次读取之前复核期限；EOF、精确预算及最终版本检查后重新检查授权、实时 scope、期限与取消。真实第二数据库连接先复现可信无 policy 兼容路径忽略末段 scope 撤销，再于实时检查加入后通过。三项公共回归覆盖这些边界；期限不能抢占锁等待本身，不返回已确认的局部或过期摘要。

当前本机 workspace 为 **619 passed / 0 failed / 13 ignored**；完整 Clippy `-D warnings`、定向 fmt、OpenSpec strict 和 release CLI/MCP/FFI 构建通过，当前 release stdio 18/18、认证 HTTP/SSE 13/13。独立代码审查在运行 37 项目标测试及外部期限/撤销探针后批准，架构审查放行最终 scope 检查并独立运行三项期限测试。源码 SHA `ae0223cecdc1710b8c4544b1c84359e1df778882` 的 Windows 两个 Rust 版本均逐项通过十二项原生内容、两项线程模式/能力、两项公开错误映射、三项期限/末段检查、公共哈希预算及九项内容比较测试；[同 SHA 的 22/22 项 CI 全绿](https://github.com/loong10k/diskgraph/actions/runs/37017346676)，包含五个原生包、八个 Rust、五个 Kotlin 和两个 Swift/GRDB 宿主。15.11 按此本地普通文件行为完成。

本增量没有新增 p50/p95 或 RSS 测量，不宣称提速；原生证据覆盖 CI NTFS 夹具。其他文件系统、真实云 provider、Linux 占位保护、原生写操作、移动 provider/包/设备、签名和生产长期运行仍分别验收。8.2、8.9、15.2–15.6 保持未完成；公开危险文件工具保持关闭，vendor 源码/pin/digest 未变，全平台生产就绪仍未完成。

## Engine 源码边界（RT-09 / 15.12）

源码提交 `9dbb6347eb8b9f00da8e1ad50bac0dd38d4cdff7` 将 Engine 入口从 2,344 行降至 67 行，最大生产模块为 320 行的扫描执行模块。唯一 Engine 继续持有状态；范围/策略/revision 授权、任务、fenced 发布、历史回收、查询/历史及容量模块在同一对象上提供真实实现。每个生产对象含私有记录、trait 与 alias 独立文件。根公开导出及 `content`、`live_evidence`、`verify` 路径保持兼容。Store 入口仍为 94 行，见 [Engine 源码图与职责映射](../crates/diskgraph-engine/README.md#source-boundaries--源码边界)。

全平台 AST 门禁先在真实多对象、超长入口及缺失中文契约上失败，再于迁移真实实现后通过；同时拒绝隐藏对象、私有空函数、限定路径占位宏、wildcard import 和未挂载文件。另一次真实 RED→GREEN 核验 `all(test, …)` 属于测试，而 `any(test, windows)` 不能隐藏原生生产对象。中文来源和参数/返回契约说明实际原生 Rust 行为，没有虚构 Java 对应。

独立迁移审计去掉文档、规范化内部限定可见性及可选签名尾逗号后，198 条类型、常量和方法记录全部一致；55 项公开模块/根导出清单一致。该核对补充编译和行为回归，不能当成通用运行等价证明。独立代码审查批准，架构审查 CLEAR，核对了 graph→control 顺序、同 guard 授权、reader/期限复用、fence/取消代次与 RAII 清理。

本机 Engine 全目标为 **105 passed / 0 failed / 5 ignored**；workspace 为 **622 passed / 0 failed / 13 ignored**。完整 Clippy `-D warnings`、定向格式、OpenSpec strict 和 release CLI/MCP/FFI 构建通过；release stdio 18/18、认证 HTTP/SSE 13/13、隔离四读者负载 4/4。[本机原始样本](benchmarks/engine-structure-load-2026-10-02.json)为 20k 文件、扫描 0.456 秒、32 次查询 p50 7.301 ms/p95 11.783 ms、数据库/WAL 37,449,728 字节；不宣称提速或生产延迟保证。扫描器 14 份上游摘要及 pin 保持不变。

[同源码原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37025049488) 已完成：**22/22 项全部通过**，包含八个 Rust、五个 Kotlin、两个 Swift/GRDB 宿主及五个原生包。Windows stable 与 1.97.0 日志逐项确认三个结构回归及十二项原生内容测试通过，macOS stable 日志也确认三个结构回归通过。15.12 按此结构增量完成。本机 macOS 的 Windows 测试目标为零项，不计原生验收。

此次迁移保留既有实时探针限制：lsof 子进程轮询有协作 timeout，但管道读取缺少严格字节/期限预算；轮询跳过不可读条目，之前条目可能表现为 Removed。拆分没有增强这些行为保证。原生写操作、真实 provider 不下载、移动 provider/制品/设备、签名与生产 soak 仍未完成，全平台生产就绪仍不成立。

## 进程证据解释（EV-06 / 15.13a，2026-10-03）

本子项改变 lsof 结果解释，不改变子进程执行。正常成功及 stdout 为空的退出码 1 均保留 partial，因为系统权限、警告及 PID 启动身份未核验；正向观察继续保留，verdict 显示观察数，空 partial 结果仍为 unknown。旧空请求的 Full 仅表示没有查询对象，不能据此证明某文件或系统无人使用。公开签名与结果类型保持兼容。

解析按字节标签与 NUL 边界处理，不在 UTF-8 字节 1 切片；非法标签/PID、缺少进程上下文和未终止字段为不可观察。查询键保留原生字节，但 NUL 模式中的 lsof 名称仍是显示字段，可包含反斜杠、caret 或十六进制转义。没有经验证的可逆协议时，潜在转义、控制/非 ASCII 字节或 deleted 标注均不匹配；其他无歧义 ASCII 正向观察继续保留。Unicode、非 UTF-8 和带标注/转义名称的身份为 unknown，不宣称已支持这些名称的真实工具观测；无法无损表示的 Windows 查询明确不可观察，也不声明原生 Windows lsof provider 可用。这是路径观察，不是打开文件句柄的身份核验。参见 [lsof 官方输出契约](https://github.com/lsof-org/lsof/blob/master/docs/manpage.md)。

先将旧后处理原样提取，四项既有观察通过；新增十项解释测试有九项实际失败。复审又补出 deleted 后缀误匹配的真正 RED，以及两个真实 macOS lsof 显示碰撞：换行变为字面反斜杠序列、字节 0x01 变为 caret-A。两个真实反例都先在当时解析器上失败，再通过拒绝歧义字段修复。人工 ASCII/原生字节夹具只能证明解析行为，不能替代特殊名称或各系统的实际工具可见性；本机与同源码 CI 的结果仅在实际执行后记录。

8.7 的默认离线完成声明已撤回；15.13 仍未完成：同时有界排空管道、累计输出/期限/取消、原生后代清理及 Git 配置隔离没有在此实现。lsof 先等待后读取、未排空 stderr，以及无界 Git 命令仍存在；unborn/HEAD/upstream 错误语义和 Git 可执行 filter 继续分别整改。本增量没有新增 p50/p95 或 RSS 测量，不宣称提速。原生写操作、provider/设备、签名和生产 soak 同样未完成，全平台生产就绪仍不成立。

本机最终 workspace 为 **636 passed / 0 failed / 13 ignored**；证据测试 18/18、Engine 结构门禁 3/3。完整 Clippy `-D warnings`、定向 fmt、OpenSpec strict 与 release CLI/MCP/FFI 构建通过；当前 release stdio 18/18、认证 HTTP/SSE 13/13。两路独立复审批准本子项。扫描器 14 份上游摘要全部一致，vendor 源码/pin 未变。

源码 SHA `533986775cee33b4d9372253178e7af67c262f2c` 的[同源码原生 CI 全部 22 项通过](https://github.com/loong10k/diskgraph/actions/runs/37033696892)，包含八个 Rust、五个 Kotlin、两个 Swift/GRDB 宿主及五个原生包。Windows stable 与 Rust 1.97.0 日志均逐项确认十三项解析回归通过；人工记录不代表原生 Windows lsof provider 已实现。macOS stable 日志确认十四项解析回归，包括真实 lsof 换行/控制字符显示碰撞测试。15.13a 仅按结果解释及覆盖语义完成，8.7 和父项 15.13 保持未完成。


## 实时证据共享执行器（EC-04 / 15.13b，2026-10-03）

Engine 内新增私有平台执行器，共用于进程及 Git 采样。新增 `ProbeLimits`、`sample_process_usage_bounded`、`sample_git_bounded`，既有公开签名及结果类型保留。兼容入口默认整次协作期限 15 秒、所有子命令 stdout/stderr 共计 1 MiB，超过 64 MiB 的配置拒绝；Git 多条命令借用同一预算，不逐条重置。两条管道在进程运行期间按固定小块排空；字节恰好耗尽仍须实际 EOF、正常退出及末段期限/取消核验。资源失败、异常退出或清理失败停止后续命令，主错误和次级清理诊断均保留。

Unix 使用自有进程组和非阻塞管道，WNOWAIT 保留 leader 至清理/回收完成；leader 离开原组时只终止仍自有的 PID，不跟随新 PGID。宿主 auto-reap 明确拒绝，外部回收则 fail-closed，主动脱离组的后代不受组约束。Darwin 仅 zombie 的 EPERM 需要两次完整成员集合一致、全部 SZOMB 及 leader 正确父身份；50 ms 有界重试覆盖退出过渡，不把 INEXIT 本身当已完成。

Windows 创建时用 JOB_LIST/HANDLE_LIST、KILL_ON_JOB_CLOSE 和固定地址 overlapped 管道，不采用运行后分配 fallback。成功零字节读不等于 EOF，意外取消是读取失败；属性列表持有参数数组直到销毁，命令/环境编码在累计超限前停止分配。Job 终止后最多观察计数一秒，未知或实际非零超期拒绝完整结果，用户 HANDLE 本身不推导计数状态。关闭 Job 提供终止后备，自有 pending I/O 只有确认最终完成后才释放；leader/I/O 安全等待可能超过观察期及采样协作期限。要求支持创建属性、受信 .exe 和绝对项目目录，拒绝相对程序路径及脚本。这是协作资源控制，不是严格 RSS 或调度 SLA。

测试先行在旧 runner 上实际产生 12 项中 11 项失败；后续补出的 Git HEAD 异常终止和 leader 逃组也先红，旧逃组清理等待约两秒。最终本机证据 suite 42/42，含 128 轮快速退出/预算/回收、真实 Git/lsof 与并发独立取消。独立绑定源码的探针确认 HEAD/upstream 信号失败阻止后续命令、逃组 leader 及时终止，以及 1000 轮结束后无未回收 child；这些是本地夹具观测，不能当性能分位数。

初次全量运行因共享构建产物消失（ENOENT）未执行部分二进制，不计通过；独立 target 重建后完成 **660 passed / 0 failed / 13 ignored**。完整 Clippy `-D warnings`、定向 fmt、OpenSpec strict 与 release CLI/MCP/FFI 构建通过，当前 release stdio 18/18、认证 HTTP/SSE 13/13。扫描器 14 份摘要一致，上游 pin/源码不变。实现阶段的原生验收保持未完成，最终源码的实际结果记录如下。

本增量没有新 p50/p95 或 RSS 结果，不宣称提速。Git 配置隔离、可执行 filter、离线/只读保证、unborn 修改与引用格式错误仍由 8.7 和父项 15.13 验收；原生写操作、provider/设备、签名和生产 soak 继续分别验收，不宣称全平台生产就绪。

首轮[原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37043023057)尚未通过：Linux stable 的脚本夹具执行前遇到 ETXTBSY，改由独立 writer 写完并退出，目标信号断言不放宽。Windows 两个 Rust 版本均为 71 passed / 1 failed，其他所有执行器原生回归通过；唯一失败是测试假定持有用户 HANDLE 必须阻止 Job 计数归零，而实际 cleanup 返回 Ok。验收改为直接核验持有外部 HANDLE 时的 signaled 状态和实际 Job 计数，另以明确注入非零计数验证有界失败；注入覆盖不冒充真实外部引用故障。生产观察策略仍依据实际查询，最终源码原生门禁继续保持未完成。

源码 `ddf6e934bebddf89783039f92d40e1f0cdf76d66` 的[第二轮原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37044718593)为 20/22 项通过。Windows stable 的 74 项 Engine 单元测试全部通过，随后 Clippy 拒绝测试模块后的辅助函数；修正只移动该函数，不改变实现。macOS Intel stable 的期限夹具假定首条 150 ms 命令在 270 ms 内完成，在负载下失败。新回归先完成轻量命令，再耗尽真实的整次五秒预算，最后确认第二条命令返回 Deadline 且未创建 marker。它去掉了 120 ms 启动假设，仍以可观察副作用检查后续命令不能重置整次期限。最终源码原生 CI 仍是必需门禁。

源码 SHA `ef0ba765fc48db3c8fbd27d1f3dc1f4e6bb363e4` 的[同源码原生 CI 全部 22 项通过](https://github.com/loong10k/diskgraph/actions/runs/37046771798)：八个 Rust、五个 Kotlin、两个 Swift/GRDB 宿主、五个原生包，加上格式及 vendor 门禁。Windows 两个 Rust 版本日志均记录 Engine 单元测试 74 passed / 0 failed，逐项确认创建时句柄隔离、两管道 pending 清理、后代清理、外部持有句柄、零字节写入和注入的计数失败回归通过。Windows stable Clippy 通过，Rust 1.97.0 按现有 CI 策略跳过此步骤。macOS Intel stable 为 Engine 单元测试 69 passed / 0 failed，新的共享期限回归通过。15.13b 按本次协作执行边界完成；Git 配置/错误语义、8.7 和父项 15.13 保持未完成，不代表全平台生产就绪。

## Git 可观察语义（EC-02/04 / 15.13c，2026-10-03）

候选保留公开签名和 GitSample 字段，要求 Git 2.46+ 的明确引用存在性接口。HEAD 保留直接 symbolic 目标，已存在的悬空/损坏引用不能降级成 unborn；unborn 与 detached 仍报告真实修改。porcelain v1 NUL 记录逐个计入未跟踪文件，rename/copy 仅计一个状态；非法记录、OID、数量及溢出均报错。本地跟踪数据缺失仍是 unknown，不说明已推送。

stash 存在时仅支持 files 后端。Git 只定位 metadata common 根，代码追加固定 logs/refs/stash，避免 Git 先 canonicalize 日志目录/leaf 链接。原始日志读取共用整次预算，核验留存 commit 与完整逆序列表，保留重复次数；合法 drop、无 rewrite 删除、expiry 和缺日志保留可见列表语义。Unix 保留初始文件版本并复核路径，Windows 保留既有原生父目录/文件租约。FIFO、链接、坏日志、预算失败及已观察变化均拒绝。这是变化检测，不是 Git 原子快照或同步内核 I/O 的硬期限；reftable stash 枚举明确 unsupported，common 根定位本身不认证 scope 或隔离配置。

测试先行先复现十项原语义失败，再补出非法 XY、悬空 symbolic HEAD、旧 stash 漏计、合法消息分隔符及日志链接预解析的真实失败；另有回归覆盖同字节改版。独立代码复审实际通过 44 项 Git 回归、三项结构门禁及 15 项隔离公开 API 探针；架构复审放行此实现边界。最终本机 workspace/协议和新源码原生 CI 结果须另记后才能勾选 15.13c。配置/filter/fsmonitor 隔离、懒取、可选 index 写入及离线/只读执行继续由 8.7 和父项 15.13 验收。不新增延迟/RSS 测量或提速声明，全平台生产就绪仍未完成。

最终本机门禁为 **701 passed / 0 failed / 13 ignored**，含 83 项实时证据测试；全 workspace Clippy `-D warnings`、定向 fmt、OpenSpec strict、release CLI/MCP/FFI 构建均通过。当前 release stdio 18/18、认证 HTTP/legacy SSE 13/13，通过 14 份上游摘要核对。ignored 不计通过；新源码原生 CI 仍待完成，15.13c 保持未勾选。

源码 `9f83a647f3d4864b6ffaa797c3e14b7c8fa125dd` 的[首轮原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37054669648)为 **20/22 项通过**。Windows 两个 Rust 版本的测试通过，但 stable Clippy 拒绝用绝对分隔符 join 构造根路径；修复按原生 Prefix/RootDir 组件构造，并明确拒绝驱动器相对路径。macOS Intel 取消隔离测试的未取消子进程退出码为 88，来自夹具的三秒 watchdog；有限次 sleep 不能保证在负载下及时结束，后续改为父进程确认另一采样已取消并清理后才释放独立子进程的握手。此失败未证明跨采样误杀，修复仍须真实验证正常退出与隔离，不能删除 watchdog 或取消断言。最终源码的原生 CI 继续是验收门禁，15.13c 尚未完成。

CI 修复后的最终本机门禁仍为 **701 passed / 0 failed / 13 ignored**；完整 Clippy、定向 fmt、OpenSpec strict 和 release 构建通过，当前 release 协议为 stdio 18/18、HTTP/legacy SSE 13/13，14 份上游摘要一致。取消隔离测试移入独立文件，原自执行子进程路径保留，各文件少于 500 行；A/B 专用夹具有独立十秒 watchdog、八秒 runner 预算，清理后首次完整心跳与后续严格递增分别有有限等待，未知不以零值兜底。Windows 根组件回归须等待实际 Windows CI，本机未执行不计通过。Git 配置隔离及全平台其余门禁保持未完成。

源码 `d699ff9c4397c1b8aac6d793198dd5d8d7325a45` 的[同源码原生 CI 全部 22 项通过](https://github.com/loong10k/diskgraph/actions/runs/37057731602)，无 skipped 项。Windows stable 与 Rust 1.97.0 日志逐项确认两项根组件回归与取消隔离通过，各为 Engine 单元 **103 passed / 0 failed**；stable Clippy 通过。macOS Intel Engine 单元 **110 passed / 0 failed**，新取消隔离回归通过；Intel 原生包的实际 archive、升级/回退与打包后二进制验收也通过。15.13c 按 Git 可观察语义完成；这不是后续配置执行限制、私有视图或全平台验收证明。

## Git 懒取和可选写入限制（EC-04 / 15.13d1，2026-10-03）

在唯一命令入口固定前置 `--no-pager --no-lazy-fetch --no-optional-locks`，所有 HEAD、status、stash 及 upstream 命令采用同一策略，保留公开签名、原环境、整次期限/累计字节/取消与错误传播。对象不在本地时拒绝，不能由 Git 自动向 promisor remote 获取；普通 full index 的可选 stat 刷新不落盘。旗标与现有 Git 2.46 最低版本兼容，见 [官方参数契约](https://git-scm.com/docs/git/2.46.0)。

两项真实 RED 分别确认旧采样修改了内容未变文件的 index 字节、触发缺失 HEAD 的本地 remote-ext helper。后者先由直接 cat-file 实证 helper 能实际启动，再清除 marker 测试采样，只有受控测试程序写标记并退出，没有实际远端联网。固定旗标后两项与专用 helper fixture 3/3 通过；保留普通 dirty=0、index 字节/mtime 与最终无 lock 断言。原错误/信号夹具明确检查并移除固定前置旗标，非法 OID/计数、HEAD 变化和异常退出断言保留。独立代码复审通过 policy 3/3、resource 3/3、semantics 19/19；架构复审通过全部 Git 47/47。当前本机 workspace **704 passed / 0 failed / 13 ignored**、实时证据 86/86，Clippy 与格式通过；release/协议与新源码原生结果须继续记录，15.13d1 未勾选。

该子项不阻止仓库 filter/fsmonitor，也不阻止 Git 读取 split index 时刷新 shared index 时间；不认证无其他源 metadata 写入、完整离线或只读执行。D20 私有视图仍须实现。真实审查还复现较新私有 index mtime 导致假 clean，精确保留原 mtime 才恢复修改观察；不能以关闭危险配置后返回假状态替代安全实现。父项 15.13、8.7 及原生写/provider/移动设备/签名/生产 soak 门禁保持未完成，不新增 p50/p95/RSS 或提速声明。

最终 release CLI/MCP/FFI 构建与当前 release stdio 18/18、HTTP/legacy SSE 13/13 通过，OpenSpec strict 与 14 份上游摘要核对通过，vendor pin/源码不变。源码 `06011ddc885c9e0227220d0851cd8a9fbf61b38f` 的[同源码原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37059408918) **22/22 全部通过**，无 skipped 项。Windows stable/1.97.0 分别为 Engine 106 passed，Linux stable 111 passed，macOS Intel 113 passed；逐项核对两项真实 policy 回归与 helper fixture 均通过。15.13d1 据此完成；15.13d、15.13、8.7 及全平台其余门禁仍未完成。禁止源 stat 缓存的可选更新可能增加后续重复内容检查，该性能取舍尚未测量，不能宣称优化了延迟。

## Git 私有执行视图（EC-04 / D20，实施中，2026-10-03）

准备阶段现将受支持的配置、index 和引用复制到私有执行视图。原生元数据读取拒绝链接、特殊文件、来源变化及超预算输入；index 预检验证 SHA-1/SHA-256 完整摘要与受支持布局，保留普通 assume-valid 标志及原 index 的精确 mtime。私有配置不包含 fsmonitor 外部命令；配置过的 filter driver 不含执行命令，实际使用时按 required 明确失败。include、split/sparse index、gitlink、replace/grafts、特殊后端和 promisor 等不能保真的语义明确拒绝。普通、linked、shallow、CRLF、忽略、rename、racy-index 及 SHA-256 夹具比较实际 Git 状态，不引入伪造 clean 的兜底。

独立审查先复现再修复三处额外边界：相对 PATH 项使切换工作目录后选择仓库伪造 Git；长期保留祖先目录句柄使 64-FD 子宿主第七次捕获耗尽；精确名称名单在大小写不敏感卷丢弃 CONFIG/INDEX。现工具只从绝对 PATH 目录解析一次，目录记录捕获后释放句柄，原生别名明确拒绝。64 个记录捕获及复核后 FD 回到初始值。显式完成会拒绝私有目录删除失败，保留主错误及次级清理诊断，清理后再核验期限和取消。Windows DACL、HANDLE 计数及阻塞删除原生回归已加入，但不计为本机执行通过。

初版 `011fe7f` 实施检查点的本机 workspace 验证为 **762 passed / 0 failed / 13 ignored**；这是功能证据，没有新增 release 延迟、峰值 RSS 或提速测量。该阶段仍先由 Git 读取宿主系统配置，缺少实际临时分配及卷剩余容量门禁。下述后续修复补齐这两处实现缺口；源对象库递归 alternates 仍不是严格访问范围/输入上限。单次路径解析仍临时使用 O(depth) 句柄。新源码原生 CI 及全平台其余门禁仍须完成，不能用此前 06011dd 的 CI 验证本次实现。

最后本机门禁还包括 workspace Clippy（警告拒绝）、定向 workspace fmt、OpenSpec strict、vendored 扫描器摘要、release CLI/MCP/FFI 构建；当前实际 release stdio 18/18、认证 HTTP/legacy SSE 13/13 验收通过。独立代码复审在 Git 105/105 及结构门禁通过后批准该本地增量，明确不批准未闭合 D20 或原生平台边界。按已有授权提交推送后运行新源码 GitHub CI。

## Git 输入、私有容量与完整性后续修复（EC-04 / D20，2026-10-03）

系统配置发现现于空私有配置上下文中运行固定路径 printer，取得绝对原生路径后先捕获内容、再解析，共用元数据字节/条目、期限及取消预算。Git、安装包 Shell 和绝对 PATH 目录仍是受信宿主依赖。隔离旧源码夹具先在 FIFO、超预算及畸形宿主输入上失败；当前 macOS 配置测试 12/12 通过。Linux 非 UTF-8 夹具及 Windows 行为仍须原生 CI 验证。

私有目录创建、文件写入和 index 时间保真现统一进入一个 owner。增量账本使用原生对象分配量（Unix blocks 或 Windows AllocationSize），默认上限 128 MiB、卷可用余量 64 MiB。无法确认计量、特殊/替换对象、未登记文件和超预算均拒绝采样。原生容量测试 13/13 通过，覆盖分配量与逻辑长度不同、条目上限、目标竞态及父目录链接拒绝；这些测试覆盖原生 owner，尚未验证 public sampler 小配额拒绝。报告分配不包含未归属的文件系统全局元数据，余量检查不会预留空间。

两项公开采样/清理回归先复现了私有 index 在长度及分配量不变时被修改仍获接受、替换的外来根目录被误删。普通私有文件现于 owner 关闭写句柄后保留完整原生身份/版本，被动终检只比较、不刷新水位；清理核对登记根身份，保留主错误和清理诊断。最新完整性测试 3/3 通过（含 helper 夹具）。身份检查与按路径删除不能原子隔离同权限竞态；原路径不存在不证明移走的 owner 数据已删除；目录历史也不是持续审计。

前一源码 `7f2bf111` 的[原生 CI 终态为 20/22](https://github.com/loong10k/diskgraph/actions/runs/37068347873)，两个 Windows 测试任务在既有目录 NtCreateFile 处报错误 87，其余 20 项通过。后续修复在打开既有元数据目录时移除不兼容的目录专用创建选项，再通过返回句柄校验类型及身份；新目录仍要求独占目录创建。这一修复须由新源码 Windows 原生测试确认，本机 macOS 通过不能代替。

Store 入口仍为 94 行，只声明和导出模块。56 个生产源码文件现增加 AST 门禁，拒绝 500 行及以上的文件；最大文件是 410 行的任务存储。Store 全目标 82 项通过、5 项明确 ignored。行数门禁辅助职责和事务边界复审，不能作为架构质量分数。D20 及父项继续未勾选，等待其余验收证据。

冻结后的本机门禁为 workspace 全目标 **790 passed / 0 failed / 13 ignored**；完整 Clippy（警告拒绝）、定向 fmt、OpenSpec strict、14 份 vendor 摘要及 release CLI/MCP/FFI 构建通过。实际 release stdio 18/18、认证 HTTP/legacy SSE 13/13 通过。两路独立复审确认修正后的完整性/计量路径，各自排除自己编写的文件。这些是本机功能结果，不是新增性能测量或全平台生产验收；新源码 CI 结果另行记录。
