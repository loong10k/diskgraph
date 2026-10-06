# Unix 清理失败的所有权与容量语义

本项属于 15.13 原生回收门禁，不完成该父项。

## 验收场景

真实子进程退出、管道 EOF 后，被宿主外部 waitpid 消费原 leader。原 owner 检测到 ECHILD 后必须拒绝基于旧 PID/PGID 的清理，后续重试也不得报告成功。转入恢复 registry 后，每次 drain 必须保留原失败及已占用槽位，不允许新的 reservation 复用该容量。

## 当前证据

既有真实外部回收测试加强后 RED：第二次 cleanup 错误返回成功。移除丢失等待权分支的 cleaned=true 后 GREEN；三次直接重试与三次 registry drain 均保留失败及容量。macOS 原生子进程回归 40 通过、0 失败，唯一默认忽略的隔离辅助夹具由真实测试显式执行。原始日志和源码摘要见 docs/benchmarks/unix_lost_wait_ownership_2026_10_06。

本项不证明 Linux 原生执行、首次检查之后的 wait 竞态、有限时间回收或全平台生产就绪。

## 后续末段 wait 竞态

原先的完成分支还会在末段 wait 报错后置 cleaned=true。既有回归增加初检之前和末段 wait 之前两个真实外部回收时序。末段使用仅测试编译的线程局部单次 checkpoint 调用真实 waitpid，不模拟 ECHILD。原代码实际 RED；修复后 wait 的 ECHILD 撤销旧数值身份权限，保留原错误及容量，其他 wait 错误保留 owner 重试。macOS 40/0/1、结构6/0通过，日志见 docs/benchmarks/unix_cleanup_late_wait_2026_10_06。进程组终止失败路径、Linux 和有限时间回收仍未完成。

## 组终止失败的身份锚点

组终止报错时不得继续 wait 并消费原 leader。即使 leader 已退出，它的未消费等待权仍须保留到组终止确认；registry 保留原 owner 和容量。测试使用真实已退出 child，线程局部持续注入 EPERM，原代码在 waitid WNOWAIT 身份断言实际 RED。修复后连续三次 drain 保留同一 leader 与槽位，解除故障后实际回收并允许再次 reservation。该故障注入验证错误状态机，不代替真实内核权限拒绝验收。macOS 原生41/0/1、结构6/0通过。

macOS 冻结候选599源只覆盖 unix_child.rs、unix_child_group.rs 和 unix_normal_exit_tests.rs，其余字节不变。门禁必须实际执行41项并包含外部等待竞态、组失败保留及原非正PID隔离证明；旧40项不能通过。挂载13/0，本项等待原生CI，不完成15.13父项。日志见 docs/benchmarks/unix_group_failure_retention_2026_10_06。

## macOS 单次期限内恢复合同

ScanWorkerRecovery::drain_until 接收宿主绝对期限；到期或锁竞争返回 false，不取 owner、不发送信号、不消费 wait。锁外单次处置 fresh 私有 session：实际原组终止后，只有完整原组退出观察和原 leader 非阻塞 wait 均完成才能返还容量。活动、到期、原生权限/等待错误和 panic 都将原 owner 放回原槽；外部 ECHILD 不允许重试旧数值组。不得 sleep/retry-loop 或调用阻塞 waitpid 作为这条路径的实现。OS 单调用和归还责任所需短状态锁没有硬墙钟保证。原 drain 兼容接口仍在，有限前端退出不据此宣称完成。

期限入口暂接既有legacy drain时真实过期请求RED为0/1；修复后目标通过。原组失败/末段外部wait/连续槽容量保留、原生与标准WNOHANG缓存、panic和锁竞争回槽均通过。最新并行native_child44/0/1、真实父驱动7/0、结构6/0；原并行正常组查询不完整失败也保留，不因精确和串行复查通过而删除。原日志及源码摘要见 docs/benchmarks/macos_deadline_recovery_2026_10_06。Linux期限恢复、前端有限退出、默认安装扫描和全平台严格检查仍未完成。

## Linux 当前提交验收来源

Linux Engine 原生门禁必须测试 CI checkout 对应的完整当前提交，不得先把旧冻结宿主补丁装配进当前源码。构建前核验所有受版本控制的构建输入与 HEAD blob 一致，拒绝已修改、暂存、未跟踪输入和符号链接；保存提交、逐文件 SHA-256 和清单摘要。构建后再次核验输入未变化。历史装配工具只用于显式历史候选，不能替代当前提交生产验收。原 namespace 隔离、普通 UID、实际 worker 摘要与原 init wait 门禁保持。

## Linux 单次期限恢复验收

Linux 的公开 Recovery::drain_until 必须沿用出生时原 pidfd，不使用数值 PID 重新打开或发信号。真实活动 child 在期限已过时，连续调用不得关闭控制输入、发送信号或消费 wait；原槽不可重用。真实线程 seccomp 拒绝 waitid 时，连续恢复必须传播原 EACCES 并保留同一 owner/容量，未过滤宿主随后可用原 pidfd 实际消费等待。正常清理只有实际 P_PIDFD wait 消费成功且原整个线程组退出后才返还槽位，测试以保留的原 pidfd 重复 wait 得到 ECHILD 和 POLLIN 为证。每轮仅使用 WNOHANG/零超时 poll，不进行内部 EINTR 循环、sleep 或阻塞 wait。前端有限退出仍须独立完成，不能由该接口推出。

三个 Linux runner 的 RED（37438383770、e839fdb）均仅因缺失 drain_until 接口出现 E0599；这是原生编译的接口缺失证据，不称为运行时行为 RED。现接入原 pidfd、零超时轮询和单次 WNOHANG，新增3项用例必须与原5项一起实际通过且保留原wait消费标记。macOS共享registry回归3/0、结构6/0、fmt通过；Linux运行结果仍待CI，有限前端退出和生产父项不勾选。

Linux 期限恢复已通过当前完整源码原生门禁：37438978565 / c98dc1b，x86 stable、x86 Rust1.97、arm64 stable 各8/0/0（原5项+新增3项），逐项包含期限容量保持、真实seccomp wait拒绝、原pidfd实际消费标记和原namespace init wait闭环。原始source清单/worker摘要/日志见 docs/benchmarks/linux_deadline_recovery_2026_10_06/verified-native.json。Clippy -D clippy::all通过但21条既有Rust警告未消除；严格warnings、前端有限退出和全平台生产父项仍未完成。

## 完整工作区门禁复核

4255fc8 的本机完整工作区回归实际运行111个顶层目标：1516通过、431失败、23忽略，33目标失败；按每个Cargo目标最后的结果计数，未将隔离子夹具的内层结果重复累计。GitHub同提交Linux/Windows严格Build失败，原始日志记录dead-code错误，macOS相关任务仍排队。专项门禁不能抵消该全量失败。测试专用LinuxScanImageError只应在test配置编入；历史Rust pre_exec安装方法无调用者，当前生产seccomp保持由原子出生C路径使用同一BPF program安装。移除闲置入口不完成严格质量或生产父项。

原全量运行中未提供独立协议驱动夹具，造成5项原owner错误处置用例失败。通过当前Cargo example明确生成夹具并按实际artifact注入DISKGRAPH_SCAN_DRIVER_FIXTURE后，该组6/0/0通过。CI新增相同构建/摘要/来源绑定步骤；只供测试，不设置产品镜像环境值，不改变安装信任、请求权限或默认扫描Unsupported状态。严格Build仍有未完成项，不能以该局部复查抵消原431项失败。

## Linux 生产与测试接口编译边界

3ce2aaa 的 Linux 严格 Build 原始失败仍有8组未使用接口错误。验收要求：只供测试的 UnixChild 旧启动接口及 LinuxAtomic 的测试观察接口不进入生产；Linux 产品保留 spawn_checked 探针、LinuxAtomicChild 控制协议、实际 pidfd 等待和期限恢复，macOS 已编译原生路径不关闭。不得增加 allow/dead_code 或降低 -D warnings。UnixNormalExit 在 Linux 仅由测试构造，生产不存储其无效状态；私有 from_spawn 的恒定参数移为内部初始化，不改变公开 API。

本机受影响 native_child 回归44/0/1、结构门禁6/0/0、cargo check 与 fmt 通过；原始日志见 docs/benchmarks/linux_test_boundary_2026_10_06/。Linux 严格生产构建和当前完整源码原生8项门禁待CI，尚不勾选生产父项。

## 当前 Linux 密封组件验收来源

f148e49 的三组 Linux 当前源码专项8项均通过，常规CI严格Build已通过；sealed-image步骤却因历史候选覆盖后重复声明4模块而失败，后续全量Test未运行。修复要求：隔离git archive保留当前提交源码，不拷贝历史候选、不新增模块声明；记录实际5文件摘要，缺失或重复声明应在Cargo前拒绝。保持原11项原生测试和严格warnings，不能跳过该门禁。新增3项来源回归因缺少current_sources真实RED，修复后须GREEN并保留实际原生11项结果；不据此完成全平台父项。

## Linux CLI/MCP 实际 worker 部署验收

6fef42b 的三组Linux严格Build与当前密封组件11项通过，实际CLI index返回unsupported；原因是验收未配置受信扫描宿主。CI须从同提交Cargo实际bin artifact构建生产worker，记录SHA/长度/提交，不选择协议驱动夹具或邻接清单，使用既有本地部署配置入口。不改变Engine::open可信库构造语义、远程授权或macOS固定安装要求；保留CLI/MCP真实索引及结果断言，未运行全量测试不能宣称通过。CI生成预期仅证明受控源码构建部署，不代表发布安装信任或全平台生产完成。

17694e7 / CI37442529819 的 Linux stable 与 Rust1.97 实际CLI/stdio/认证HTTP验收步骤均completed success，严格Build通过，完整workspace Test正在运行；步骤状态原件见product-176-step-status.json.gz。只确认此实际部署下的产品流程，不代表全量或Windows/macOS生产通过。Windows严格编译的HANDLE仅用于cfg(test)原句柄见证，将导入限定测试，不屏蔽尚未集成的原生清理能力；本机结构6/0、fmt通过，Windows原生编译仍待验证。

## 验收镜像与 Cargo 输出生命周期

17694e7全量回归仍失败37目标，直接Engine::open的旧扫描夹具无宿主，CLI子进程启动还返回conflict。Cargo全量测试可重建同一路径bin，部署不能继续指向可被重写的target输出。验收镜像须从实际Cargo bin artifact原句柄独占复制到runner临时路径，完整摘要/长度绑定副本，复制前后身份与高精度时间一致，超限/源变化/目的已存在明确拒绝；协议example不得供应产品镜像。3项副本回归先RED后GREEN（后续Cargo替换、拒绝协议example、禁止覆盖），保持生产授权、密封与原生执行机制。全部旧Engine夹具迁移和全平台父项尚未完成。

## 原生身份/进程夹具显式宿主

17694e7原始全量日志确认Linux epoch两案在run_job因Engine::open没有扫描宿主而失败；执行/观察夹具同样未提供宿主。保持生产构造函数语义，以NativeScanEngine显式绑定CI独立镜像预期、原Config预算和公开open_with_scan_worker，夹具保留外部Recovery直到结束，错误/panic仍实际回收。迁移epoch、execution、observation三目标，不改opaque handle、hardlink、重开、PID身份及占用断言；缺部署材料必须失败。既有actual worker专项夹具的显式环境也绑定同一固定副本。Linux原生验收待CI，本机仅能验证源码/编译边界，不能据此勾选平台父项。

继续迁移7个需要实际扫描的Linux集成目标（engine_flow、content_flow、compare_two_trees、growth_narrow_read、scan_options_parity、verify_comparison_content、relation_request_budget）。这些目标通过同一真实宿主夹具持有Engine和Recovery，原测试函数、参数、权限/预算/结果断言不变；其他平台保持既有构造路径，不伪造平台支持。Linux实际结果须原生CI确认，更多旧单元/集成夹具仍未迁移，不能由这7个目标替代workspace门禁。

f2c3b1b的Linux stable完整workspace原始日志确认失败目标37→29，CLI各目标不在失败清单，镜像固定副本后的真实产品回归保留通过；仍不能把剩余旧库夹具Unsupported称为已通过。此次7目标本机macOS仅no-run编译通过，结构6/0与fmt通过；不称Linux运行验收。原始全量日志及目标末段汇总见workspace-f2c-linux.log.gz与workspace-f2c-summaries.json.gz。


## 共享查询与进程预算夹具的真实宿主

355f383 / CI37444199873 的 Linux stable 原始全量日志确认失败目标降至18，此前迁移的扫描目标未出现在失败目标清单；不能由该改善宣称全量通过。当前仍失败的9个Engine目标（query_request_budget、query_finalizer_budget、query_until_budget、history_size_eligibility、history_compatibility_matrix、process_preparation_budget、process_entry_scope_budget、hardening、process_job_dispatch）使用旧构造入口。共享夹具改为Linux显式受信宿主，保留全部业务函数和断言；嵌套模块所需测试类型可见性改为crate，Engine/Recovery先于临时目录释放，非Linux构造保持既有路径。

本机9目标no-run编译、结构门禁6/0、排除固定vendor的既有workspace格式命令通过；全workspace格式包含vendor会报告上游原始格式差异，未修改vendor。Linux运行待下一提交CI，Windows原生清理未接入生产导致严格Build失败、Linux安装包实际索引Unsupported均继续开放，不降低门禁。原始355全量日志与本机记录见linux_test_boundary_2026_10_06/workspace-355-linux.log.gz和native-nested-*.log.gz。


## Linux 解包产品扫描部署

355f383原生Linux package日志确认release构建成功，解包CLI实际index因未配置宿主返回Unsupported。包验收在Linux必须先取得同提交实际Cargo release bin artifact的独立预期（沿用独占快照部署），解包worker经既有非链接/同句柄/字节预算读取核验与该预期相符后，四条外部验收流程统一执行包内worker。不得从包内清单生成信任、缺配置不得回退；Rust生产宿主仍执行原严格镜像验证。此处只表示受控CI构建信任，不替代发布签名、安装信任或macOS/Windows原生部署门禁。

新增6项回归真实RED（缺少部署接口）后GREEN，追加子进程环境传播共7/0；组成9/0、原文件准入5/0、独占副本3/0通过。本机强制Linux分支的组成9/0仅验证脚本分支与全部四次调用参数，不称Linux原生执行；旧组成mock因新增deployment参数真实失败后更新，保留所有归档/字节/清单断言并增加包内路径和独立摘要断言。原生解包stdio/HTTP/升级/20k负载须下一提交CI实际运行后才能验收，所有父项保持开放。记录见linux_test_boundary_2026_10_06/package-*.log.gz。
