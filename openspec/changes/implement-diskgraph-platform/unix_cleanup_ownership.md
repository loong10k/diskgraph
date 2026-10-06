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


724d3c1 / CI37445312905 的Linux x86_64（112208764848）与ARM64（112208764904）package原生job均终态success，原始日志逐项确认解包stdio18、HTTP13、upgrade/rollback7、controlled load4。独立构建预期+包内实际worker验收已经运行通过，不等于完整20k/200k性能、发布安装信任或全平台生产。Windows package构建失败、macOS package仍queued；状态和原始日志见package-724-native-results.json、package-724-linux.log.gz、package-724-arm.log.gz。

cb39286 / CI37445008522 的Linux Rust1.97完整Test终态failure，失败目标18→9，新增9个Engine共享目标未出现在失败目标清单；仍失败Engine lib、FFI lib、Ops lib，以及MCP durable_request_authority/git_evidence_contract/git_evidence_socket/git_status_scope_contract/hardening/process_evidence_contract。原始日志见workspace-cb-linux-1_97.log.gz。Linux stable/ARM全量仍运行，不能以1.97的目标改善替代全量成功，也不能将9目标计作9个独立漏洞。


## MCP 集成扫描宿主与恢复外槽

剩余六个MCP目标的真实扫描不能由旧McpService::open/open_remote隐式提供宿主。夹具经显式open_with_scan_worker/open_remote_with_scan_worker接入Linux CI同提交实际worker及独立预期，保留本地/远程认证分支、原2M节点配置、真实socket/runner和原数据库；唯一Recovery持有至整个Fixture结束，重开服务时同时替换服务与外槽。其他平台仍使用既有构造，不用Linux材料伪造支持。测试外槽当前阻塞drain不代表生产有限退出已完成。

本机六目标no-run编译及既有排除vendor的格式检查通过，201个原assert宏片段与HEAD逐项对照不变；实际macOS六目标11通过/26失败，明确保留未配置可信扫描宿主的Unsupported，不以编译或授权拒绝通过代替实际扫描。本机全部运行及首次失败记录见mcp-native-fixtures*.log.gz，Linux实际运行须下个提交原生CI验证。Windows生产句柄清理、macOS安装扫描及全平台父项继续开放。


## Engine 单元夹具显式扫描宿主

Linux单元测试仍有旧Engine::open扫描构造；11个测试模块及GitEvidence/ProcessExecution共享夹具改为显式原宿主与唯一Recovery，Linux容量重新打开的执行用例沿用同一真实构造。既有集成夹具与单元夹具复用同一个实现，生产Engine::open/授权/发布/协议不变；Linux-only测试模块声明不构成默认产品部署。原夹具owner字段先于独占目录释放，持续保留原清理责任。

首次结构检查因跨声明目录path失败，随后额外extern crate声明也被入口门禁拒绝；改为普通test-only模块及父模块类型导入，不修改门禁。最终结构6/0、本机Engine lib no-run和排除固定vendor的格式检查通过，14个修改测试文件的原assert片段逐项不变。此处本机不编译Linux分支，不称Linux运行已通过；MCP d2e8130原生CI仍运行，FFI/Ops扫描夹具、Windows生产清理与macOS安装仍开放。记录见engine-unit-host-*.log.gz。


## Ops真实Arc及原扫描预算可变借用

55b061a原生Linux三lane严格Build真实RED：job_authorization与scan_observation两个既有用例需要独占可变Engine修改原scan_budget，夹具仅Deref导致E0594和unused-mut。补测试夹具DerefMut返回原Engine，不改变两个用例、期限或生产API；原Linux编译仍须下一提交验证。本机结构6/0及原格式门禁通过。

Ops Project保留真实Arc<Engine>交给原PlanBuilder/Executor，在Linux另持唯一恢复外槽至Project结束，仍使用原100k节点额度和独立部署材料；没有包装Arc替代公开接口、没有启用CLI/MCP危险写工具。增加本workspace scan-worker的dev依赖，Cargo.lock仅增加该已有本地crate依赖边，不升级外部包。原249个assert片段保持不变，本机offline check、lib no-run、结构7/0、fmt通过。Ops实际Linux业务及原写适配语义须原生CI，不据此宣称平台写能力通过；macOS20条既有Engine警告未掩盖。原日志见engine-unit-55-native-compile-red.log.gz及ops-scan-host-*.log.gz。FFI、Windows原生清理、macOS安装及有限前端恢复继续开放。


## FFI旧库授权夹具与Linux严格构建

d2e8130 / CI37446156365 的Linux stable全量原始日志只剩3个失败目标（Engine/FFI/Ops lib），六个MCP实际目标已脱离失败清单，原日志见workspace-d2-linux.log.gz。4c6f208三Linux lane严格Build均success，确认原预算可变借用编译修复已在原生环境生效；整次Test仍运行，不能将Build当行为验收，状态见engine-4c-native-build-status.json。

FFI authorization两项通过Engine真实扫描准备旧库的用例改为Linux显式受信宿主+独立外槽，保留旧库归属证明、非UTF-8和歧义拒绝全部13个原assert片段。公开scan_native_json、NativeService、异步导出、wire、授权服务及ABI未改；其中公开扫描仍不能由这些夹具通过来宣称可用。新增本地scan-worker dev依赖及单条lock依赖边，没有升级包。实际旧库准备的Linux结果须下个提交CI；本机offline check、FFI lib no-run、结构8/0及fmt通过，既有macOS Engine20条警告仍未隐藏。记录见ffi-legacy-*.log.gz，其他FFI公开扫描失败、Windows清理、macOS安装和生产有限恢复保持开放。


## Linux全量最后两类夹具接入缺口

316e66fd1929bf30dc1dee0e2f5478af67d2edaf / CI37447459176 Linux stable job112215616764终态failure，实际全量仍只有Engine lib与FFI lib两个失败目标，Ops已脱离失败清单。Engine索引Git身份用例仍用旧Engine::open；原子launcher失败注入用例要求root显式编译DG_LINUX_ATOMIC_LAUNCH_FIXTURE，但root只提供不同协议的atomic-birth镜像。最新日志有多项连带失败，不沿用4c日志三项失败数量。原日志见workspace-316-linux.log.gz。

Git用例在Linux复用现有真实NativeScanEngine，唯一Recovery保持至Engine夹具结束，全部9个原assert片段保持。CI独立编译linux_atomic_launcher_fixture.c的IMAGE_ID=1/2两份ELF，保留source/两镜像摘要，提供原测试指定环境变量；不把atomic-birth镜像别名充数，不修改故障注入、禁止回退或回收断言。两平台之外构造与生产API不变。

本机Engine lib no-run、结构6/0与workflow YAML解析通过；fmt首次因条件导入排序失败，按rustfmt只调整该文件导入后，排除固定vendor的fmt通过；本机是macOS，不能据此声明Linux分支行为通过。原始记录为engine-final-fixtures-*.log.gz。原生执行等待本次提交CI，FFI公开扫描、Windows默认产品扫描/句柄清理、macOS可信安装、有限前端恢复及完整性能门禁仍开放，不勾选父任务。


## 独立入队请求与测试验证时间

041ea863c0728bb64d549f12f3930ae1879267f9 / CI37448505312 Linux Rust1.97 job112219033046终态failure发生在前置Store入队测试，尚未进入全workspace；不得称Engine新夹具已通过。原timely create/merge测试在首请求后执行完整库快照，第二次独立请求仍沿用首请求600ms绝对期限，实际第二次调用于254行返回BudgetExceeded。原日志见workspace-041-linux-1_97.log.gz。

正控保留每请求600ms，先等首期限过期并再次调用，明确要求BudgetExceeded且完整持久状态不变、连接配置恢复，再为第二次独立请求建立其唯一600ms期限，要求仍合并到同一个原job。没有在单次请求内部刷新deadline、没有修改生产限额或去除失败断言；39个原assert片段按顺序保留，增加2项拒绝/持久状态断言。原负向写锁、COMMIT、撤权、quota和过期认证场景不改。

本机定向10/0、Store结构1/0、排除vendor的fmt与Store all-targets严格Clippy通过；全Store lib回归结果另列，不以本机结果替代Linux原生。此前性能配对脚本固定旧baseline尚无现有ScanWorker API，当前harness仍用旧Engine::open，不能从该入口声称新架构20k/200k验收；完整性能和所有全平台父项继续开放。

本机Store完整lib实际终态291通过、0失败、1 ignored（135.94s），原日志enqueue-independent-request-store-lib.log.gz保留；ignored不计通过，跨平台原生仍待提交验收。


## 共享可信部署入口的层级

生产只读CLI/MCP与既有库兼容入口应复用同一不可变部署解析及平台准入。当前ScanWorkerSettings定义在MCP，底层Engine无法复用；将真实实现与其全部测试移至Engine，MCP保持旧公开类型重导出，CLI改为底层导入。只移动实现所有权，不放宽macOS固定安装或Linux/Windows独立预期验证，不增加隐式镜像探测或远程请求选镜像能力。FFI公开扫描接入与有限恢复尚未由该层级修复完成，父任务保持开放。

Engine新导出API在原源码实际编译RED（E0432），迁移后原对象/方法正文除导入、格式和中文doc冒号外一致；原31个测试assert片段保持。原部署15/0、Engine结构6/0、MCP真实类型兼容/二进制配置/启动/结构四目标合计19/0，本机CLI all-targets check通过，既有macOS Engine20条未接入组件警告未隐藏。结构初次5/1因两个doc契约格式失败，按原要求补齐后6/0；fmt的模块顺序RED也保留后纠正。

CLI entry与binary配置6/0，旧host lifecycle用例在macOS明确违反固定安装契约而实际失败；保留Linux/Windows原成功断言，macOS实际CLI要求unsupported、ok=false且未创建数据库，新平台准入1/0。这是拒绝普通路径的负向证明，不能充当macOS固定安装成功验收。最终fmt通过，记录见shared-deployment-*.log.gz；不修改生产限额、危险能力或任何父任务checkbox。f9b1890三Linux lane前置已越过原期限夹具，当前真实全量Test仍运行，没有由观察超时重启。
