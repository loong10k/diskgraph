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

源码 `57546dfb2d1200a937577c8f63c2a4475ae29d2f` 的 [CI](https://github.com/loong10k/diskgraph/actions/runs/37073529881) 在 Windows 警告拒绝构建中发现两条仅用于 Unix 测试的导入；两个 Windows Rust 任务均未进入测试。导入现添加 `cfg(unix)`；这次仅测试代码的修正后，配置 12/12、Engine 全目标 Clippy 及定向 fmt 通过。Windows 行为仍须新一轮原生验证，其余任务的成功不能关闭该门禁。

## Windows Git 工具路径修复（EC-04 / D20，2026-10-03）

源码 `14203e1d2134ed8edd55ed12cf4718fc9d4fb281` 的 [CI 终态为 20/22](https://github.com/loong10k/diskgraph/actions/runs/37074169662)。两个 Windows Rust 版本均完成构建及只读协议验收；Engine 单元测试各为 134 passed / 34 failed。12 项原生私有容量回归、目录枚举和替换根清理已逐项通过。32 项失败发生于 Git 读取配置，另两项分别是目录测试的 `..` 被 PathBuf 预先消去、linked-worktree 夹具直接向 Git 传入 verbatim 路径；这些失败不能记为通过。

该 Windows 宿主安装 Git for Windows 2.55.0.windows.5。其 [mingw 实现](https://raw.githubusercontent.com/git-for-windows/git/v2.55.0.windows.5/compat/mingw.c) 对 `access()` 与 `fopen()` 使用不同路径校验，默认 NTFS 保护拒绝后者中的 `?`；[配置实现](https://raw.githubusercontent.com/git-for-windows/git/v2.55.0.windows.5/config.c) 对显式 GLOBAL 先检查访问、再打开。结合实际错误，私有空配置的 `\\?\C:\…` 表示是主要失败原因；原生配对测试用于进一步验证该推断。

新的唯一工具路径适配器只在严格校验后移除合法本地驱动器 verbatim 前缀，保留 UTF-16 名称、大小写和单末尾目录分隔符；拒绝点组件、ADS、保留设备名、名称改写及不可保真的命名空间。普通工具路径最多 259 个 UTF-16 单元，超限明确拒绝。Git cwd、路径环境变量、绝对 PATH/SystemRoot、配置参数、属性/忽略路径及对象库 alternate 均使用该边界；原生捕获与 owner 路径继续保持原表示及句柄身份校验，不启用宿主 Git 配置，也不关闭 NTFS 保护。两项夹具现分别保留真实词法 `..`、从普通临时路径创建 linked-worktree 后再执行原生采样。

词法门禁先在旧直传实现上 5 项失败，末尾目录分隔符回归另外确认 1 项失败；最终词法 8/8、配置 12/12、上下文 2/2 和结构门禁 3/3 通过。冻结源码本机 workspace 全目标为 **798 passed / 0 failed / 13 ignored**，Clippy、定向 fmt、OpenSpec strict、14 份 vendor 摘要及 release CLI/MCP/FFI 构建通过，实际 release stdio 18/18、认证 HTTP/legacy SSE 13/13 通过。独立代码及架构 lane 均批准该路径增量，各自排除编写过的文件；批准范围不含尚未运行的原生行为或其余 D20 能力。新增 Windows 实际 Git 配对测试核对普通/verbatim 名称的完整原生文件状态、GLOBAL 与 `--file` 行为及宿主配置不变；该测试尚未在 Windows 执行。原生 CI 结果继续记录，15.13d/D20 及全平台父项保持未完成。没有新增延迟、RSS 或提速测量。

源码 `8d1ccbabec1e2de31e32e79a3d9bb7971867495a` 的[原生 CI 终态为 19/22](https://github.com/loong10k/diskgraph/actions/runs/37077247510)。两个 Windows Engine 单元套件各为 144 passed / 32 failed；新增实际配对已经记录同一 empty 文件的 raw GLOBAL/`--file` 均退出 128，ordinary 均退出 0，其后在生产上下文构造中遇到工具路径词法拒绝。前缀表示的根因获得实测支持，但整个 Windows 采样仍未通过。另一失败是 macOS stable 的源 metadata 哨兵 before 42 → after 41；旧日志没有消失路径，不能判为夹具维护或生产写入，也不能以重跑掩盖。现保持所有不变断言，增加路径增减诊断与成功/预算失败/预取消分段比较；本机 Git 2.48.1 的目标重复 50/50 通过，仅为本机观察，不解释 CI Git 2.55.0 的失败。其他 19 项已通过，但不能关闭上述边界或 D20。

后续冻结增量将 shell PATH 视作搜索列表：只保留绝对且能安全表示的宿主目录，每项在筛选前检查整次取消/期限；私有目录、工作树、SystemRoot 与实际 Git 参数继续严格拒绝不可表示路径，固定 Git 程序不重选。构造与 spawn 的错误带独立阶段标签，不输出完整环境。Windows 子进程夹具注入点步、重复分隔符、尾点、ADS 与相对 PATH，核验实际传入的搜索列表并两次调用生产 printer；新增真实宿主 PATH 分类诊断用于下次原生定位。原生测试尚未通过，不能仅凭旧构造错误指认具体 PATH 字符串。

本机阶段诊断先红后绿，词法/阶段 9/9、system 12/12、context 2/2、结构 3/3；workspace 全目标 **799 passed / 0 failed / 13 ignored**，完整 Clippy、定向 fmt、OpenSpec strict、14 份 vendor 摘要、release CLI/MCP/FFI 构建及实际 stdio 18/18、HTTP/legacy SSE 13/13 通过。独立代码与非作者架构 lane 批准路径筛选及严格源哨兵增量。CI 另加仅 macOS 的固定 20 次源哨兵原生观察，要求每次确实执行一条回归且通过；第一次失败停止并保留诊断，单次 60 秒、step 5 分钟上限。该步不替代完整套件，不把原生 42→41 原因视为已解释。新 SHA 结果继续记录，D20 与全平台父项仍未完成。


## 原生 CI 配置兼容与夹具竞态修复（D20，2026-10-03）

源码 `ac10c416bb11f0594a20853835574466d3fa4ddf` 的[原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37079509314)终态 **19/22**。两个 Windows Engine 单元套件各 **153 passed / 26 failed**；实际普通/verbatim 配对与注入宿主 PATH 的受控 child 已通过，26 项采样错误现在明确为 `unsupported Git core semantics: core.fscache`。Windows stable 的 MCP 另有 **119 passed / 1 failed**：断线任务已完成，但夹具错误地要求新连接的首状态必须为 queued/running；MCP-05 不要求客户端观测每个中间态。最小修正允许合法 completed，并另开独立连接断言同一 job ID 的 completed 记录仍可查询，不改生产状态机或加入等待。

macOS Rust 1.97.0 的 Engine 为 **207 passed / 1 failed**，新增路径诊断捕获源 metadata 消失的路径 `objects/maintenance.lock`。Git 2.55 的[自动维护入口](https://raw.githubusercontent.com/git/git/v2.55.0/run-command.c)默认允许 detach，[维护实现](https://raw.githubusercontent.com/git/git/v2.55.0/builtin/gc.c)在后台结束时释放该锁。这一调用链与夹具 commit 后立即取水位吻合；路径差异本身不确认删除者 PID，也不能回溯证明旧 8d1ccba 的计数变化。新增 Trace2 回归先在本机 Git 2.48.1 真实 RED，观察到 commit 启动 `maintenance run --auto --quiet --detach`；随后仅给临时夹具准备命令固定 `-c maintenance.auto=false`，回归 GREEN，原生产采样与完整 path/count/bytes/mtime 哨兵不变。

CI 现在对源哨兵与上述实际失败的 SHA256 语义分别固定重复 20 次，每次必须真实执行且通过一条测试，首次失败立即停止，原有期限门禁保留。提取同一脚本的本机 **20×2** 已通过；这不是 Git 2.55 原生验证。Windows fscache 的受支持配置实现、最终冻结门禁、非作者复审及新源码 CI 结果继续记录；15.13d/D20 及全平台父项保持未完成，不增加性能或生产就绪声明。

Windows fscache 的最小增量现只将 `core.fscache` 加入既有布尔允许列表，原字段/覆盖顺序回放；依据精确版本的[布尔解析](https://github.com/git-for-windows/git/blob/v2.55.0.windows.5/compat/mingw.c#L303-L305)及[进程内缓存实现](https://github.com/git-for-windows/git/blob/v2.55.0.windows.5/compat/win32/fscache.c#L450-L545)。三项策略/真实 Git 重解析/公开采样回归实际 **0/3 RED → 3/3 GREEN**；源完整哨兵、dirty/stash/upstream 与下一次采样新增文件观察保持。未知 core、非法 bool、fsmonitor/filter 边界未扩大，配置 renderer 及公开签名未改。

最终冻结本机 workspace 全目标 **803 passed / 0 failed / 13 ignored**，Clippy 警告拒绝、定向 fmt、OpenSpec strict、14 份上游摘要及 release CLI/MCP/FFI 构建通过；实际 release stdio **18/18**、认证 HTTP/legacy SSE **13/13**。独立代码 lane 批准 fscache/Trace2/CI（排除其编写的 HTTP 测试），非作者架构 lane 另批准 HTTP/spec 及同一 Git/CI 增量。批准不含整项 D20 或未执行 Windows 缓存验收。新源码提交推送后仍须原生 CI，15.13d 及父项不勾选。


源码 `4577e1f904f08449dbfa8d8f2db6c02a1d1dd255` 的[原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37081412323)终态 **21/22**。两个 Windows Rust 任务均成功，逐项核对三项 fscache、Trace2 维护夹具及 MCP 断线完成后查询回归实际通过；macOS stable/1.97.0/Intel 三任务各源哨兵 **20/20**、SHA256 **20/20**，完整 Engine 单元各 **212/0**。这为本轮配置和夹具增量提供原生证据，不能替代 D20 剩余边界。唯一失败是 Linux ARM 的 `specialist::tests::a_probe_accepts_a_new_enough_version_and_refuses_an_old_one`（ops83/1）；旧断言未输出实际 AdapterStatus，无法确认原因。诊断补充保留单次执行、10秒/4096字节/retries0及旧版本拒绝，未改变生产探针。已有本地 Docker arm64、无网络/无新镜像/无生产挂载的受控内核探针证明继承 writer FD 可导致脚本 ETXTBSY，全部关闭后可执行；这不是本次 CI 原因证明。

后续只读源审查在同一 SHA 的可信库入口上复现 ODB 边界：A的源alternates指B及B再指C均获成功样本；准备后新建alternate也成功；**68,157,551字节**的源alternates未计入64MiB元数据额度。当前私有alternate仍指整个源objects，源info/alternates和对象文件没有进入捕获账本。共享期限/取消/管道预算有效，但不能声称对象输入或访问范围已受限。未发现CLI/MCP/FFI生产调用该公开sampler，不描述为已复现远程泄露；Windows网络文件访问风险是源码推断，未做联网或原生复证。后续须以有界私有flat ODB或真正文件访问隔离关闭递归源目录输入；禁止仅作一次源alternate检查或用改变源nlink/ctime的硬链接充当只读副本。D20/d及全平台父项保持未完成。

Unix specialist 的脚本产物现由独立 Rust 测试子进程写入、同步、关闭并设置0755；父测试不持有writer，核查child成功、实际1条测试/完成标记、精确字节/权限后，仍运行真实SandboxedRunner。新增ignored helper只供显式受控调用，不以其单独跳过表示验收。此整改消除父进程写句柄的继承窗口，保留实际失败状态诊断；原CI的具体原因仍未确认。最终冻结本机 workspace **803 passed / 0 failed / 14 ignored**（历史13另加该助手），workspace Clippy/fmt、OpenSpec strict、vendor14、release构建及实际stdio18/18、HTTP/legacy SSE13/13通过。非作者窄复审批准夹具与如实状态；新源码Linux/ARM及完整原生CI继续待验收，不勾选D20或全平台父项。


源码 `28838a3e48e5c71abd98f9c82c6bcbbff8558fd8` 随后通过[同源码全部 22 个 CI 作业](https://github.com/loong10k/diskgraph/actions/runs/37082997467)。Linux ARM 日志确认新版本接受/旧版本拒绝的真实探针已通过，ops 为 **84 passed / 0 failed / 1 ignored helper**；这验证该宿主的夹具增量，不证明前次失败原因。已复现的源 ODB 输入/范围缺口仍阻止 D20 完成。EC-04 与 D20 已补充有界私有 flat 对象副本验收：禁止源 alternate 和硬链接，共享原始输入/条目/期限/取消，复核源版本并执行私有分配/卷余量门禁；实现和新源码验收尚待。桌面只读 CI 不能关闭移动 provider/设备、原生写保真、宿主对接、签名或生产持续运行门禁。


另一个 `28838a3` 隔离公共库探针保持默认累计管道 1 MiB，并将期限设为60秒：已提交空工作树返回 dirty=0；加入12,000个空的长名未跟踪文件后，确切返回 `probe cumulative output byte limit exceeded`。这是有界拒绝，没有假 clean 或已证远程漏洞；现有20k/200k索引/查询基准不能证明 Git sampler 支持该规模。原始对象默认额度同时计算捕获和末段复核，私有分配和卷余量继续独立门禁。


## 有界私有 Git 对象副本（D20，2026-10-03）

采样现只把安全普通 loose 对象和配对 pack/index 复制进私有对象目录，不再给 Git 源对象 alternate。源 alternates/http-alternates、promisor、未知名称、链接和不支持类型明确拒绝。已知加速文件不交工具，仅 no-follow 查询类型，不读或快照其正文。对象沿用原生读取与来源集合，捕获和终态字节/版本复核共享原64 MiB/32k累计额度、期限和取消；复制走原分配/卷余量 owner，禁止硬链接。普通 packed SHA1/SHA256、linked/shallow 及 dirty/stash/upstream 回归通过。公开签名/默认值不变；小私有配额测试对共用采样实现注入资源，不表示新增公开配置API。

真实RED：旧对象目标1通过/7失败，旧资源目标1通过/3失败/1专用助手ignored，sidecar父目录替换0通过/1失败。最终对象/资源16通过/0失败/1助手ignored，受影响live evidence为201/0/1。两路非作者独立复审批准增量，均排除各自编写的原语。冻结完整workspace **821 passed / 0 failed / 15 ignored**；警告拒绝Clippy、定向fmt、OpenSpec strict、14份上游摘要及release CLI/MCP/FFI构建通过，实际release stdio **18/18**、认证HTTP/legacy SSE **13/13**。新源码原生CI仍须运行，D20及全平台父项不勾选。

另一个Q-02响应计费修复先复现debug溢出panic及release回绕接受，再以checked_add保留精确可表示上限；溢出时保留累计量并锁存ByteLimit。预算目标debug/release均 **6/6**，非作者窄复审批准；不关闭其余查询/provider门禁。

[本机复制成本原始测量](benchmarks/git_object_copy_2026_10_03.json)在同一对隔离不可压缩packed仓库使用相同release公共采样driver，每阶段首个样本另记及8个热样本；p95为nearest rank，在8项中即观测最大值。各阶段源路径、身份、字节、mode、mtime、ctime和nlink水位不变。数据只代表macOS arm64/Git2.48.1，不称提速、冷缓存、跨平台SLA或并发子进程总RSS硬限。

| Pack对象字节 | 热p50 修复前 → 后 | 热p95 修复前 → 后 | time报告峰值RSS 前 → 后 |
| --- | --- | --- | --- |
| 8,391,645 | 197.31 → 313.63 ms | 258.68 → 398.79 ms | 10.44 → 24.17 MB |
| 25,174,555 | 232.08 → 390.88 ms | 239.53 → 462.71 ms | 10.49 → 57.90 MB |

复制与内容复核随捕获字节增加，另有目录排序、路径解析和记账成本；初始对象buffer仍保留，终检可另持一份。大pack和宽status会明确超过原默认额度。该增量只覆盖可信库，未发现生产CLI/MCP/FFI采样调用。原子快照、严格RSS/调度、真实provider不下载、原生写、移动设备、宿主应用、签名和生产持续运行尚未由此验收。

源码 `830b7c8a99b7fce354a125fca7eae3d860454f74` 的 [CI 终态为 20/22 成功](https://github.com/loong10k/diskgraph/actions/runs/37086380783)。两个 Windows Rust 任务的 Engine 均为 **192 passed / 4 failed / 1 helper ignored**：四个新资源 wrapper 在 child 夹具的 `git init` 配置读取时失败，尚未执行预算或清理断言。源码将 canonical Windows 临时根经 child 临时目录环境传入配置文件路径，verbatim 路径拒绝是有源码支持的候选原因，仍待原生双路径对照。窄夹具修复保留 canonical 身份，只给临时环境使用现有验证过的工具表示，额度、源水位、清理及实际执行 marker 均保留。本机 Unix 目标 **4/0/1**，修复源码的 Windows CI 尚待；本次失败及其 20 项成功都不能完成 D20 或全平台门禁。

随后 [dbc1f05 的 CI](https://github.com/loong10k/diskgraph/actions/runs/37087667994) 仍为 20/22。两 Windows 夹具现已通过 `git init`，但新增 raw 路径负向对照的 `git config --list` 返回 128/`fatal: error processing config file(s)`，原断言只接纳初始化阶段的措辞。窄修仅接纳这两种已观察的配置错误，并保留退出码要求；普通路径必须成功并读出唯一完整 NUL 标记，raw 若成功也必须读出同一标记。资源、源水位和清理断言不变，在 Windows 尚未到达。本机目标与独立复审通过，仍须新原生 CI。

后续 FFI 结构只读审计确认入口 1,343 物理行，约 737 行生产代码、606 行内联测试。锁定 UniFFI 0.32.2 将模块路径和导出文档纳入元数据/checksum，C 符号名却只用 crate 名，所以移宏到普通子模块再 Rust 重导出不足以证明旧 Swift/Kotlin 绑定兼容。已在本机冻结 `830b7c8` release 库及从它生成的旧绑定：全部 118 动态导出、23 元数据符号和 19 个实际执行的 checksum。独立编译器实验确认 root `include!` 保留宏模块路径，但实际库等价和旧绑定交叉加载门禁尚未实现。另已确认条件中文 rustdoc 属性不改变普通编译的宏文档；它不等同用户要求的无条件中文 `///`。本次未改 FFI 生产源码，不声明结构或 ABI 已整改完成。

[26b6f1c 的 CI](https://github.com/loong10k/diskgraph/actions/runs/37090184096) 终态 21/22。两个 Windows Rust 任务逐项通过四个资源/源水位/清理 wrapper，Engine 均 **196 passed / 0 failed / 1 helper ignored**。唯一失败步骤是 Windows stable Clippy，针对 cfg(windows) 测试的 `set_readonly(false)`；该语句现加局部测试 lint 许可。[Rust 官方文档](https://doc.rust-lang.org/std/fs/struct.Permissions.html#method.set_readonly) 区分 Windows 只读文件属性与 Unix 写权限位，Unix 夹具仍只补 owner 写位。没有改变生产行为或全局 lint 策略；冻结新源码的原生 Clippy 以及 D20/其余平台门禁尚待。


## 操作源码边界（OP-14 / 15.14，2026-10-03）

Ops 入口从 2,406 行降到 83 物理行，62 个生产文件保留各职责的真实实现，最大 373 行。根路径与 `specialist`、`docker` 公开路径保持兼容；唯一 Executor/CrossVolumeCopy 状态持有者、原锁、事务、批准版本及显式失败清理保留。中文注释写明真实原生 Rust 来源与实际参数/返回。Ops README 和双语架构图说明边界；没有增加运行时服务、所有权层，也没有性能提速测量结论。

原多对象入口在新增 AST 结构门禁中真实失败。后续 union 反例在补 Visitor 前实际 5 通过/2 失败，补齐后 7/0，两项 mutation 共用真实门禁。门禁遍历平台分支，只精确放行既有不支持平台的零资源 discard 空清理；构造/发布仍拒绝，不将该空清理计作平台写能力。独立规范化源码对照确认 103 个生产、222 个含测试函数体，有效平台条件及 impl header 不变，86 项解析后公开 API 路径等价，不需函数体或限定路径豁免。两路非作者复审批准本增量，最终窄复核再确认注释、union 检查、架构图和冻结 70 文件摘要。

最终本机 workspace 全目标 **828 passed / 0 failed / 15 ignored**；Ops 单元 **96/0/1**，结构 **7/0/0**，三项真实宿主操作测试仍 ignored。workspace Clippy 严格零警告、限定包 fmt、OpenSpec strict 与 14 份上游摘要通过。重建后的 release CLI/MCP/FFI 及 Ops 库构建通过；实际隔离 release stdio **18/18**、认证 HTTP/legacy SSE **13/13**。运行中构建产物及部分依赖缓存消失，最终按锁文件恢复并对重建产物执行验证；先前缺二进制和缺依赖的失败不计验收。扫描器 pin、源码和摘要未改。

新源码仍须同 SHA 原生 CI，包含 Windows stable 测试语句局部 Clippy 修正；15.14 因此继续未勾选。D20、移动端/provider/真机、原生写保真、宿主打包签名与生产持续运行门禁也保持开放。源码整改不启用 CLI/MCP 危险工具，不代表全平台生产就绪。


后续源码 `7c41ffd45714df5695e0bb88893c5981dd4645d0` 的[原生 CI 终态 17/22](https://github.com/loong10k/diskgraph/actions/runs/37092918161)。三个 Linux Rust 构建拒绝显式测试导入 CrossVolumeCopy、LiveItem、identity_of，两个 Windows Rust 构建拒绝 executor_transfer 的 describe 导入；均在严格零警告 Build 阶段失败、未进入测试，不计原生行为验收，也不是已复现运行时缺陷。窄修按原有实际使用点加导入条件：三条测试导入限 macOS，describe 限 macOS/Linux，capture_source 保持共同回归可用。函数/测试条件及函数体、公开 API、零警告策略不变。本机 Ops **96/0/1**、结构 **7/0/0**、三项宿主 ignored，Clippy/fmt 及重建 release Ops 通过；非作者窄复审批准两个导入区域与冻结源码。修正源码仍须新原生 CI，15.14 与全平台父项保持开放。


下一轮 [eae4f06 的 CI](https://github.com/loong10k/diskgraph/actions/runs/37093532369) 终态 **20/22**。三个 Linux Rust 任务均成功，ARM 日志确认 Ops **84/0/1** 与全部七项结构回归实际通过。两个 Windows 任务进入测试编译后失败：根 Windows 测试缺少 capture_source 显式导入，specialist Windows 测试多余 Path/Duration/Instant 导入。窄修只改两个测试导入区，测试属性与主体和失败源码逐字节相同，没有增加 skip、allow、根导出或 helper 可见性变更。独立复核完整迁移的平台测试依赖，检查父模块实际导出与私有 helper 的明确路径；规范化函数体相等不能证明名称解析成功。本机受影响回归及严格 Clippy/fmt 通过，Windows-only 测试本机未执行。修正源码的 Windows 构建/测试/Clippy 及完整原生 CI 仍待；不能用 20 项成功代替 Ops 结构或全平台完成。


最终源码 `7a0d1cf2d694b12c640339229c262ad8d7b2e13b` 的[同源码 CI 22 项全部通过](https://github.com/loong10k/diskgraph/actions/runs/37094332636)。两个 Windows Rust 版本实际执行两项迁移后的不支持能力测试与全部七项 Ops 结构回归；Ops 单元均 **7/0/0**，Engine 均 **196/0/1**，stable 构建、测试及严格 Clippy 通过。Linux/macOS Rust、真实语言宿主、原生只读包、格式及 vendor 任务也成功。15.14 至此完成 Ops 源码边界增量；Windows 测试验证明确拒绝，不代表原生写能力，没有开启危险工具。D20 及其余全平台能力/生产验收继续开放。本记录明确已验证实现 SHA，后续仅文档提交不改产品源码。


## 有期限的计划选择（OP-15 / D22 / 15.15，2026-10-03）

PlanBuilder 现通过一个已授权的独立 reader 精确读取所选节点，不再物化完整 revision。实际修订所属 scope 校验、逐节点/路径/live-stat 的首错顺序、超范围节点行为、重叠处理、源证据和持久计划格式保持兼容。图元数据阶段在修订查询前固定1,000ms协作期限，最终实时授权返回后再次复核；等待控制锁不能把过期结果变为成功。同步文件系统/控制调用不能被抢占，后续源证据不属于该元数据期限。CLI/MCP 危险工具仍关闭。

NodeRow 现传播全部 SQL 列解码错误，并拒绝旧 JSON 节点 ID 与 SQL 行 ID 不一致；合法可空字段、旧快照和未知大小继续支持。精确节点、children、分页与历史读取全部接入。真实 RED 分别复现非法类型静默默认、旧 ID 不一致被接受及未选坏节点阻断有效计划。最终目标测试为 **7 项计划通过 / 1 项 release 基准明确 ignored**、**8 项节点回归通过**，目标 Clippy 严格零警告通过。另一负向控制只移除最终期限检查，公开 builder 便错误接受真实1.1秒授权锁等待（**0/1 RED**）；恢复实现后通过。两路非作者代码/架构复审批准。15.15仍须完整 workspace 和新 SHA 原生 CI 后才能完成。

[配对 release 原始数据](benchmarks/plan_metadata_2026_10_03.json)保留每点全部九次观测、首次候选及仅 lint 调整后的最终重测。各阶段复用同一对隔离图/控制数据库与实际所选64字节源文件，20k/200k指合成元数据行数。每点新子进程运行九次真实公共计划并持久化；首样本另记，p50/p95使用八个热样本，nearest-rank p95即这八项最大值。成功计划跨阶段累计，控制库未重置；子进程 RUSAGE_SELF 峰值不包含父进程夹具构造。测量仅代表macOS aarch64/Rust1.98.1，不是实际文件扫描、冷缓存、跨平台SLA或严格内存上界。

| 元数据行数 / 所选节点 | 热p50 修复前 → 最终 | 热p95 修复前 → 最终 | 子进程峰值RSS 前 → 最终 |
| --- | --- | --- | --- |
| 20k / 1 | 12.973 → 1.133 ms | 13.793 → 1.467 ms | 86.05 → 9.33 MiB |
| 20k / 16 | 13.829 → 2.066 ms | 14.516 → 2.639 ms | 86.50 → 9.64 MiB |
| 200k / 1 | 103.790 → 1.059 ms | 107.544 → 1.298 ms | 606.52 → 9.33 MiB |
| 200k / 16 | 102.987 → 2.137 ms | 111.687 → 2.234 ms | 591.50 → 9.53 MiB |

实际 SQLite 指令计数在20k和200k两规模均为：**选择1项59步、选择16项944步**。这证明精确节点读取量随选择变化，不随完整快照规模变化；不能称整个计划O(M)，既有重叠过滤仍为平方复杂度，源捕获和锁等待还有独立成本。七项计划回归目前限macOS/Linux，Windows编译及store测试不能被描述为运行了这些计划测试。

## Git 原生验收补充（EC-04 / D20，2026-10-03）

新增隔离宽工作树回归创建8,192个普通空文件，确认真实status NUL输出 **1,220,608字节**。公共采样入口在默认累计1 MiB输出预算下明确失败，源路径/内容/mtime水位不变且私有目录清理；对同一工作树只提高可配置输出额度至2 MiB后完整返回8,192项dirty。受控child要求实际完成标记；隔离mutation给生产预算额外2 MiB后，该回归真实失败，不能被误计为覆盖成功。

四项可移植回归使用含空格/Unicode路径的临时原生编译callback：实际Git先执行fsmonitor、已应用clean和process filter正控制并写marker；公共采样不得启动它们，已应用filter明确unsupported而非假clean。固定测试shim运行真实Git status后改私有index一字节：原生完整身份、长度和分配仍相等，但末段必须拒绝版本变化，观察到的私有index路径必须已删除。四项在本机macOS通过；隔离撤去相关guard后四项均真实失败，非作者独立复审批准。源哨兵只覆盖路径/内容/mtime，不是完整inode/ctime/nlink；前三项callback不各自证明全私有树清理，宽status有其独立完整临时目录检查。

新 SHA 的Windows/Linux实际执行仍待CI，包含Windows Unicode callback及原生完整文件ID/分配观测；独立child现以固定相对/绝对PATH完成真实仓库同名程序正控制：必须启动工作树伪二进制并观察marker；从其他caller目录调用公共采样后必须保持可信绝对Git、marker不出现且源水位不变。parent要求确实1项测试和完成标记。本机恢复实现1/1 GREEN，另一个独立artifact故意在bootstrap后重选工作树程序0/1 RED；此mutation不描述为原生产实现。Windows真实名称搜索仍待CI。这些测试不启用写操作，也不完成设备/provider/宿主/签名/生产持续运行门禁。D20/15.13d及全平台父项保持开放。


最终本机全目标验证为 **844 passed / 0 failed / 18 ignored**。首轮完整运行843/1/18：唯一失败来自未修改的Ops AST结构门禁，新增测试使用`#[path]`但门禁按默认模块路径寻找文件。现将两个自有测试移至标准模块目录，benchmark字节和实际模块名不变，没有加门禁豁免或改生产行为；恢复结构7/7、计划目标7/7与完整suite后通过，非作者代码窄复审批准路径/lint修正。

workspace Clippy严格零警告、定向fmt及独立原生helper格式、OpenSpec strict、14份上游摘要、release CLI/MCP/FFI/Ops构建通过。重建制品真实隔离stdio **18/18**、认证HTTP/legacy SSE **13/13**。新增三项ignored为两个由wrapper显式实际执行的child夹具，以及另行已执行的release基准；其余真实宿主/provider夹具不计通过。测量源码及冻结原生测试摘要再次一致。此本机冻结仍须记录新SHA原生CI和最后非作者shadow复审，15.15及D20暂不勾选。
