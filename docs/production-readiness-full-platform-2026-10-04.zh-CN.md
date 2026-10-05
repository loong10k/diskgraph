# 全平台验收续篇 — 2026-10-04

> 2026-10-06 当前状态：全平台生产验收未完成。以下历史通过记录仅适用于各自源码与范围，不能替代当前集成版本验收。

Windows 冻结候选 `d957e66` 的 [原生验收 37384689462](https://github.com/loong10k/diskgraph/actions/runs/37384689462) 编译成功，四项 pending I/O 跨线程回归实际通过；三项原 owner 清理恢复测试实际失败，确认过早标记完成、丢失句柄与恢复槽位释放问题。[原始证据](benchmarks/windows_cleanup_native_red_2026_10_06/summary.json) 已保留。候选 `436ebea` 修复失败后保留原句柄并重试真实 OS 观察；[修复验收](https://github.com/loong10k/diskgraph/actions/runs/37385193355) 已实际通过 7/7（原四项 I/O 与三项清理回归），[GREEN 原始证据](benchmarks/windows_cleanup_native_green_2026_10_06/summary.json) 已保存。出生后 panic 的外部所有权以及清理协作期限仍未解决，不据此启用 Windows 扫描。

macOS [ARM/Intel 原始失败 37381931634](https://github.com/loong10k/diskgraph/actions/runs/37381931634) 发生于创建非法 UTF-8 测试目录，返回 EILSEQ，尚未进入扫描。[原始失败证据](benchmarks/macos_installed_invalid_filename_failure_2026_10_06/summary.json) 已保留。修正候选 `66c445a` 在同一 CI 临时卷明确检查 EILSEQ，再以合法 Unicode 文件名运行完整扫描；独立真实序列化测试保留非 UTF-8 原始路径字节且本机精确 1/0/0 通过。[ARM/Intel 新验收](https://github.com/loong10k/diskgraph/actions/runs/37385115407) 尚未完成。

最新完整 CLI/MCP 回归仍有失败，参见[原始回执](benchmarks/product_full_regression_2026_10_06/receipt.json)。尚缺统一提交的全 workspace、三平台产品正向链路与规模性能验收；资格快照通过不能作为发布版本通过。


仅 D45 暂存区隔离候选的完整 workspace 已通过 **1674/0/18（101 个结果块，exit0）**，随后仅规范排序 root 聚合声明；业务源码不变，最终严格 workspace all-target Clippy、自有九包 fmt 和 OpenSpec 校验通过。workspace 全量 fmt 的上游既有差异保留，14 份 vendor 摘要一致。非作者独审限定批准 D45，Windows 真实身份/撤权与通知竞态仍待 CI，不关闭 27 父门禁。扫描候选不在该提交范围；其首次 Mac 正常退出真实 2/5 失败及 getsid(SZOMB)=ESRCH 诊断均已保留。

D45 请求级负向见证已接入 Engine 普通 revision 与显示读取，保留真实 SQL 授权及原期限；已知撤权为拒权，连接代次失效为冲突。当前本机 Store lib **291/0/1**、Store all-target 严格 Clippy exit0；Engine revision 过滤回归 **25/0，399 filtered**。该阶段临时排除三项尚缺 API 的 Unix B 测试模块声明，随后字节恢复，不是完整 Engine 通过。Unix namespace 能力仍为 None；Windows 真实身份、通知顺序及原失败断言仍待原生 CI。候选未提交，27 父门禁保持开放。

Unix控制输入候选已真实通过新通道9/0、旧checkpoint2/0、旧probe19/0、源码规范4/0及限定Unix fmt。独立源码复审核对5个owned与3个shared摘要，限定批准非阻塞、单块所有权、SIGPIPE防护和失败清理。新API尚未进入扫描driver，非test库6条dead_code保持，不宣称严格Clippy通过；Windows候选未在本机原生执行。正常整组许可、Engine接线与可信安装仍未完成；本轮候选未提交，不关闭父门禁。

增量协议提交 `f9311dd` 的 [CI37254775910](https://github.com/loong10k/diskgraph/actions/runs/37254775910) 已终态 **20/22成功**，Windows stable/MSRV仍在两项末段撤权断言返回BudgetExceeded，四次撤权调用124264–195921µs。此前Mac准备、Mac运行资格和Linux基线失败本次未复现，不能据此称根因修复。Linux完整两轮交替配对已取回并逐项核对stdout/stderr摘要：20k宽目录候选/基线扫描比0.874与0.946，200k为0.885与0.888；300深目录为1.492与1.104，存在变慢观察（约0.103→0.154s、0.092→0.101s），不隐去。RSS差异不足以证明改善。70份本轮原始文件已归档；这是同runner已有缓存的两轮结果，不是冷缓存、性能SLO、隔离helper总内存或全平台验收。**27个父门禁仍开放。**

诊断提交 `c73fa6e` 的 [CI37253306085](https://github.com/loong10k/diskgraph/actions/runs/37253306085) 已终态 **17/22成功**。Windows stable/MSRV 的四次真实 grant/scope 撤权调用耗时135090–170427µs，超过原50ms末段窗口，仍返回 BudgetExceeded；仅定位总撤权调用耗时，未测 SQLite 内部 flush/lock 阶段。macOS ARM MSRV 的原400ms迁移准备耗时405786µs，剩余0、未进入BEGIN；macOS Intel的Cancel未取得合格运行进度且未发送动作，exit0/End1，因此不是取消行为验收。取回Linux配对原始产物后，200k**旧基线**在run_job返回Business(Conflict)，没有完整性能对比。全部失败及44份性能产物已归档，原期限、FULL和错误断言保持，未验证修复。**27个父门禁仍开放。**

增量执行 v2 协议候选已通过最终本机 workspace **1672/0/18（92 个测试套件+9个 doc-test 块，exit0）**及严格 workspace all-target Clippy。worker 共79项，新增目标24项包含两项真实 helper stdout→解码器→关闭控制输入→EOF/leader wait。早期接口缺失和仅测试代码的 Clippy 失败均保留，只作等价的显式 artifact 分支修正。独立源码审查绑定最终24项摘要。该结果证明本机协议行为，不证明 Engine 启动、可信安装、正常整组/Job退出、20ms组装、严格RSS，也不修复下方原生失败。**27个父门禁仍开放。**

最新已提交 helper runtime `1314087` 的 [CI37250991412](https://github.com/loong10k/diskgraph/actions/runs/37250991412) 已终态：**19/22 任务成功**。Windows stable 与 Rust 1.97.0 的末段 scope/grant 撤权两项断言均返回 `Business(BudgetExceeded)`；macOS Intel stable 的非法控制测试未捕获合格运行进度、未发出动作，worker 输出一个 End 并正常退出。失败尚未修复。诊断候选保留原期限、错误和断言，本机 Engine 规范1/0、授权投影4/0、Git 边界13/0通过；Windows 专属诊断仍待原生执行。一次完整规范检查因进行中的 worker 模块尚未挂载而失败，原日志保留。没有关闭父门禁，**27项仍开放**。

最新诊断提交 `b9e73fa` 的 [CI37246447522](https://github.com/loong10k/diskgraph/actions/runs/37246447522) 已终态，**19/22任务成功**。macOS ARM stable的迁移准备耗尽原400ms窗口，未进入BEGIN；Windows stable/MSRV末段scope/grant撤销实际返回 `Business(BudgetExceeded)`，未满足原拒权断言，具体耗时阶段未知。Windows MSRV合法linked-worktree夹具还返回 `scoped Git source changed before data access`，变化字段未记录。原期限和错误断言保持，终态原始日志保存在扫描基础回执中。**27个父门禁仍开放。** 本机未提交helper运行时55项通过、严格all-target Clippy通过；Engine错误传播现有7项目标通过：末发布案例先真实复现keeper的SQLite BUSY被返回为PermissionDenied，再由同代取消分支窄修复通过。独立源码审查批准9份当前Engine源码，严格workspace Clippy及fmt通过；此前完整库410/0/3在第七案之前，修正后的完整workspace通过 **1648/0/18（86 test suites，另9 doc-test blocks，exit0）**。vendored扫描器自身124/0/2通过，原pin、摘要和选项一致性检查保持。首次源码规范失败已归档，仅补齐六处注释返回标记而不改行为，四项规范检查通过；最终源码Clippy/fmt及OpenSpec验证通过。这些本机结果不证明Engine/helper整合或原生生产就绪。

授权投影候选仅将有界展示和共享末检的四处完整 scope 读取改成实时撤销标量，公开 scope() 及其他旧完整入口保持。真实请求在原50ms/256KiB下成功且两次实际授权允许后，合法2MiB展示/卷字段的 Rust requested 分配由20,973,344降至1,079字节；不是RSS或通用吞吐。实际 RED **3/1** 后原样测试 GREEN **4/0**，独立 scope/grant 回调撤销仍拒绝；本机 workspace **1621/0/18（81 test suites，另9 doc-test blocks，exit0）**、严格 Clippy 和 Engine fmt通过，独立源码审查批准。这不证明历史 TUI 失败原因。详见[授权投影回执](benchmarks/scope_authorization_projection_acceptance_2026_10_05.json)。

扫描基础提交 `792fb4c` 的 [CI37242905214](https://github.com/loong10k/diskgraph/actions/runs/37242905214) 已终态，**20/22 任务成功**。macOS MSRV Migration gate 失败：原400ms窗口准备用了407385µs，调用剩余0，守卫5µs内返回BudgetExceeded、未进入BEGIN。Windows MSRV 的 `explicit_git_sync_wait_uses_the_persisted_publication_revision` 同样失败，返回 `unsupported`；该输出不足以区分入队、捕获或复验，原因未知。两个原始失败分别保存在扫描基础回执和授权投影回执的共用日志档案中。已推送 `7f2df79`，包含已审授权投影及完整迁移串行门禁；[CI37244358774](https://github.com/loong10k/diskgraph/actions/runs/37244358774) 已终态，17/22任务成功。Linux x64 的200k性能夹具和Linux ARM历史夹具均在扫描run_job阶段返回Conflict；macOS Intel的零分配文件容量夹具拒绝原生空间计数；Windows stable/MSRV的末段scope/grant撤销返回语义断言失败，原日志没有实际返回错误。各根因尚未确认，整体原生验收未通过。本机串行门禁完整执行全部注册用例 **289/0/1**、零过滤，原期限不变；这不证明旧平台失败已经修复。27个父门禁保持开放。

扫描进程基础候选已完成共享 OS-child 的保真提取和有界协议库；尚无 helper 可执行程序、正常整组回收、控制管道或 Engine 接入。协议 **39/0**、源码规范 **4/0**、严格 workspace Clippy、自有包 fmt 及上游摘要检查通过，独立源码审查通过。真实分配负控已先失败后通过：2MiB 名称拒绝路径的 Rust requested bytes 从约2MiB降至86字节；剩余额度不足且保留已写前缀的场景降至1033字节，不是峰值 RSS。

最终全 workspace 实际为 **1616/1/18（80 suites，exit101）**：原 TUI 50ms 用例未进入 paint 即 BudgetExceeded。本次校验与 fmt 有重叠，但原因未确认，不据此排除失败。只增加阶段诊断后的单次目标 **1/0**，进入 paint 为649µs；原期限、120ms超时和实际 backend 断言保持。单测成功不代表旧失败修复；新源码原生 CI 尚待验收，旧 b3b3746 的绿色结果仅属于下方历史源码。**27 个父门禁继续开放**。详见[扫描基础验收回执](benchmarks/scan_process_foundation_acceptance_2026_10_05.json)。

源码 `b3b3746` 的 [CI37237018524](https://github.com/loong10k/diskgraph/actions/runs/37237018524) 已完成，**22/22 任务成功**。下载核对八份原生编译器回执：**64 个合格案例**，Cargo 产物来源、探针自身位置的生命周期／Send／Sync 诊断及摘要一致；Windows 源码摘要仅对应精确 LF→CRLF 转换，不误称原始字节相同。每条原生 workspace 真实执行四个作用域 owner 运行用例各一次、原期限 running merge 各一次及原 50ms 正向 reader 各一次。workspace 汇总：Windows stable/MSRV **1434/0/16**，macOS ARM stable/MSRV 与 Intel **1573/0/18**，Linux x64 stable/MSRV 与 ARM64 **1592/0/18**，均70 suites；不重复计入迁移重跑或测试清单。此前 Windows reader 失败本轮未复现，其耗时原因仍未确认。证据见[原生作用域 owner 回执](benchmarks/scoped_owner_native_acceptance_2026_10_05.json)。

完整平台清单仍为 **140/167 完成、27 开放**。本轮仅验收原生 Rust 类型约束与协调层／manager 回收；pinned 扫描器物理退出／读取隔离、受管语言宿主、provider／设备及签名门禁继续开放。下方阶段记录保留各自当时的状态。

作用域 owner 候选 `29b1ec30` 将本轮新增的 Rust tuple 构造改为生命周期约束的回调能力，由私有栈守卫独占真实 manager 句柄。旧 API 的安全静态 TLS 存储实际编译成功，新外部生命周期反例明确拒绝；这是结构约束回归，不是 Windows 死锁运行复现。提前 Drop 仍真实 finalize，忘记能力及 callback unwind 不会遗弃 manager。已通过本机 FFI **93/0**、workspace **1573/0/18（70 suites）**、严格 workspace Clippy、九包 fmt、OpenSpec、**19/19 实际 ABI 校验值**及 **8/8 合格编译案例**，实现与 CI harness 均通过独立审查。八条原生测试配置已接入精确 Cargo 产物编译门禁，新源码原生 CI 尚待运行。基线 `7e64340` 的 CI 终态 **20/22**：macOS MSRV 入队调用前期限耗尽，Windows MSRV 首次正向 engine_flow reader 在成功消费及末段授权后被最终 50ms 期限检查拒绝，耗时分布仍待诊断。两 Windows Rust 版本的此前 TLS 与 keeper 案例均实际通过；该基线结果不作为作用域 owner 候选的验收。整阻塞入口直接在 TLS／DllMain／UI 调用、扫描器物理退场／读取隔离、正式语言宿主及全平台父门禁仍开放。证据见[作用域 owner 回执](benchmarks/scoped_owner_acceptance_2026_10_05.json)。

下方阶段记录保留各自当时的状态。

原生 CI 暴露剩余资格失败：`ddd1aeb` 终态 **21/22**，`7ff7d52` **20/22**。Windows MSRV 的 TLS 夹具调用线程未在观察期限前启动，没有观察到产品 join 提前返回；Windows stable 另有 Engine keeper 测试未到发布边界，底层结果仍待诊断。仅测试候选 `55bee133` 在进入 TLS 前启动并确认参与线程，保留原期限／主动释放／真实 join，本机 FFI **89/0**、fmt、严格 Clippy、构建和独立审查通过，实际 Windows CI 尚待验证。[Rust 1.97 TLS 说明](https://raw.githubusercontent.com/rust-lang/rust/1.97.0/library/std/src/thread/local.rs#L98) 支持 loader-lock 机制判断，仍需原生验证资格。宿主 TLS／DllMain 中 finalize 是另一结构性风险，不由本次夹具修正解决；作用域生命周期 API 与扫描器物理隔离仍开放。

内部 Engine runner 候选 `41abc8f6` 已通过真实 TLS 退出 **3/0**、代次信号 **4/0**、FFI **89/0**；index tree `24d288db` 隔离导出的 workspace **1569/0/18（70 suites）**、九包 fmt、严格 workspace Clippy、构建、OpenSpec 和 **19/19 ABI 校验值**通过，最终五源独立审查通过。首次权限 wire 回归及错误表名夹具失败均保留，不误称最终成功或额外行为 RED。协调线程正常／异常退出实际 join 内部 runner，typed 拒权与原取消信号仅绑定其成功认领代次。该历史本机验收随后对应 7ff7d52 的原生 CI 37230714544，终态为 20/22，通过项及失败分类见上方记录。上游扫描器物理退出、源读取隔离、真实受管宿主和平台父门禁仍未验收。

FFI 协调层候选 `7787b661` 从 index tree `efee4d08` 隔离导出验收：workspace **1562/0/18（70 suites）**、九个自有包 fmt、严格 workspace Clippy、构建、OpenSpec 和 **19/19 旧 ABI 校验值**通过。原 30 毫秒 TLS drain 在 40.107 毫秒返回 pending，随后主动释放；不代表严格调度上限。已实现 Rust 宿主所有权，但内部 runner、上游扫描器物理退出、真实宿主及平台父门禁仍开放。下一批内部 runner 回归实际复现正常完成／撤权／unwind 三条路径提前返回（**0/3/0**）；其源码不包含在本候选中。上一提交 `fd71b50` 的[原生 CI](https://github.com/loong10k/diskgraph/actions/runs/37226948350) 已成功。证据见[owner 回执](benchmarks/managed_ffi_owner_acceptance_2026_10_05.json)。

下方为历史阶段记录。

待提交源码另以 Git index tree `76b632c6` 隔离导出，排除未完成 FFI owner，完整 workspace **1545/0/18（70 suites）**、九包 fmt、严格 workspace Clippy 与 OpenSpec 通过；这是本机验收，新提交原生 CI 尚未运行。

扫描缓存提交 `f435582` 的 [CI37222271476](https://github.com/loong10k/diskgraph/actions/runs/37222271476) 已终态 **22/22 成功**。同机、同夹具 release AB/BA 配对固定 `2a2f828`，12 份 Linux 测量具备阶段资格：20k 宽目录耗时降低约 5–7%，200k 降低约 6–11%，300 层子树基本持平。67 份产物文件摘要及有符号存储增量已核验。这是配对观测，不代表通用吞吐、冷缓存、原子快照或严格 RSS 保证。原持有目录身份缓存仍每次重读当前完整祖先链，上游扫描器 pin 和源码字节保持不变。

后续仅增加诊断的 `1fa36ba`，[CI37222674248](https://github.com/loong10k/diskgraph/actions/runs/37222674248) 已终态 **21/22 成功**。macOS stable 的 writer 探针准备阶段实际耗时 409801 微秒，超过原 400 毫秒期限；守卫在 2 微秒内正确拒绝，没有执行 BEGIN。该失败不能验收目标 writer 锁阶段，原始证据已保留，未放宽原 400/550 毫秒断言。

未提交的 FFI 生命周期候选已复现真实阻塞：30 毫秒 coordinator drain 等待 TLS 夹具的 60 秒救援后失败（0/1/0）。`JoinHandle::is_finished()` 后同步 join 不足以保证限时等待。Engine/Store 取消桥随后修复了登记等待期间漏消费停止信号和改写独立身份冲突错误的问题：登记真实 RED 0/2、身份真实 RED 1/1，修复后目标 10/0，广回归 **882/0/13（36 suites）**、两包 fmt/严格 Clippy 和独立审查通过。这些候选结果不能验收 PF-06，也不证明上游扫描线程物理退出。原始证据单独索引于[生命周期档案](benchmarks/process_job_foundation_acceptance_logs/archives_lifecycle.json)，清单仍为 **140/167 完成、27 开放**。

下文保留历史失败和当时待验收状态；其中 CI 与缓存性能的旧待定描述以以上已核验结果为准。

`2a2f828` 的 [CI37216693860](https://github.com/loong10k/diskgraph/actions/runs/37216693860) 已终态：20／22 job 成功。三条 Linux Rust lane 各 workspace 1537／0／18、70 suites，原父目录替换负控与兄弟目录变化正控均通过，确认 `5c9985b` 的三平台原生 RED 已转 GREEN。macOS Intel 的后续 Migration gate 入队权限夹具失败，Windows MSRV 的提交后恢复及 MCP 重连查询失败，整体 CI 仍未通过。恢复夹具现加强真实已提交阶段及原 token 自然到期见证；状态查询新增同一原期限内的控制锁等待，短暂竞争不立即误报预算。本机整仓 1528／0／18、70 suites、九包 fmt、严格 Clippy、OpenSpec 及独立审查通过；新原生 CI 尚待完成。20k／200k／300 深目录配对 release 性能验收脚本已准备，尚无性能结果；深子树不等同于更深的注册根链，不宣称原子快照、冷缓存或严格 RSS。


`2e33519` 的 macOS stable 原生 job111472086096 已实际成功，完整 workspace 1523／0／18、70 suites。Linux 在测试之前遇到回调生命周期编译错误，`5c9985b` 用独立类型别名恢复旧借用约束；该修正的原生 CI 正在运行。macOS MSRV 原队列10项和guard4项通过，新增host40诊断在BEGIN前准备窗口已过期；仅修正该夹具的阶段资格，本机诊断2项及Store库283／0／1通过，保留550ms总耗时断言。整体平台验收仍未完成。


D44 锁等待候选完成本机验收：22 项目标测试通过，完整 workspace 1523/0/18（70 suites），fmt 与严格 Clippy 通过，独立静态审查批准。真实准备期中断回归先在新候选失败、旧实现通过，最小修复后通过；BEGIN/COMMIT 仅重试普通 BUSY，保留原期限、有效期内中断错误、提交事实及消费者单次执行。原生 CI 待执行；Linux 扫描祖先替换新增测试仍待原生 RED，不宣称该能力已修复。


本文延续[全平台实施记录](production-readiness-full-platform-2026-10-02.zh-CN.md)，完整平台目标仍未完成。

## 最新阶段：D42 进程身份与准备预算修复，原生门禁仍未通过

当前清单为**167总项／140完成／27开放**，包含广泛父项，不等于27个独立漏洞。Git任务8.7与持久请求授权任务15.20已在实现的Index／Sync／Git路径验收。D41 修正源码原生验收已记录于下文，D43 同源码原生验收也已记录于下文；任务13.6已完成全项源码与实际原生日志复核；进程／应用collector8.6／15.13、provider、原生写、GUI／移动端／真机、签名和生产部署保持原要求。

此前80d0622的原生CI已终态：八条Rust lane失败，其余十四项通过；实际细节与保留原始日志见下文。Process身份／准备修复仍为未验收候选。本机只读设备盘点显示选中 `/Library/Developer/CommandLineTools`，标准应用目录未找到Xcode，adb可用但没有连接Android设备；这仅是环境事实，不代表移动端构建或真机验收，未安装SDK或执行设备任务。 [设备盘点记录](benchmarks/full_platform_device_inventory_2026_10_04.json)。

新提交`addf9a84d664225e58162295aac6dac911b045cf`的[CI37210806407](https://github.com/loong10k/diskgraph/actions/runs/37210806407)已终态15项成功／7项失败。Windows stable完整workspace1374／1／16、70 suites，仅VM准备INSERT中断，断连查询本次实际通过，不代表旧偶发根因已修复。三条Linux完整Test各1526／0／18、70 suites，进程身份／预算／原expiry／scope回归实际通过；MSRV整个job成功，stable两job随后仅因Linux专用Clippy失败。三macOS串行隔离仍超原墙钟断言；Windows MSRV的queue10通过，unwind在准备INSERT时中断，尚未到指定panic／cleanup。此观察不是整体验收，原始阶段日志保留于[D42回执](benchmarks/process_job_foundation_acceptance_2026_10_04.json)。

Process公开入口新增回归实际为2通过／3失败：合法scope的display与volume各2MiB时，整次调用累计Rust分配申请62,918,098字节；缺少IndexWrite或MetadataRead时分别申请8,389,002与4,194,465字节后仍正确拒权。问题是预算前不必要的完整scope复制。相同测试修复后5/5通过，三个对应整次调用累计申请降为1,363／104／16字节；Store新投影7/7、全部目标298／0／5与Engine库385／0／3通过，非作者静态复审批准。本机完整workspace为1514／0／18（70 suites），fmt／严格Clippy／完整构建／OpenSpec及14份vendor摘要通过；新原生CI仍待验收。该值不是峰值RSS，macOS的Unsupported结果也不是Linux原生验收。原始日志与冻结测试见[D42回执](benchmarks/process_job_foundation_acceptance_2026_10_04.json)。

`fd9330e44318c15db7a9a3ea0cb34e6da2b0e81d`同源码[CI37187379023第二次attempt](https://github.com/loong10k/diskgraph/actions/runs/37187379023)已终态**22/22 success**。第一次attempt的21项成功保留原执行时间，仅Kotlin Intel作为job111395457676实际重跑。首次失败发生在Java／JNA宿主执行前的Maven插件描述解析，日志不能证明网络或缓存根因；同SHA重跑实际取得Maven BUILD SUCCESS并通过会话／分页／轮询／v1／release／重开宿主验收。[D40回执](benchmarks/mcp_service_layout_acceptance_2026_10_04.json)保留两次attempt、原始失败、源码摘要与真实原生日志。

| fd9330e实际workspace日志 | 通过／失败／ignored | suites | 实际通过Git用例 |
| --- | --- | ---: | ---: |
| Windows stable | 1248／0／16 | 59 | 157 |
| Windows Rust1.97 | 1248／0／16 | 59 | 157 |
| Linux ARM64 | 1376／0／18 | 59 | 159 |
| macOS Intel | 1387／0／18 | 59 | 159 |

四份原始workspace日志还各实际执行持久授权39、既有查询／候选32、迁移MCP21、结构9项，逐案恰好一次成功。分组存在重叠，不能相加为独立用例总数；Windows仅缺两个已核验的Unix-only Git用例。观察只取workspace执行阶段，排除清单、迁移重跑及sentinel重复。此证据支持8.7／15.20，不认证新的D41源码或全部平台能力。

## D41 查询目标准备：同源码原生验收完成

合法大snapshot ID复现旧TUI／历史消费者建立账本前已经拥有必要目标的成本。17源修复先借用准入owner字段、授权实际server/scope，再准入必要snapshot ID；双侧历史与后续快照／节点／合并／计划窄读沿用同一原始`QueryReadBudget`。初始TUI准备失败不进入paint，不提交缓存部分帧。双方历史owner均已授权后，目标／消费者失败仍执行双侧授权末检，编码后成组实时grant检查保留；原期限、Complete导航与已绘明确截断画布合同不变。

```mermaid
flowchart LR
    O["借用owner字段准入"] --> A["实际scope授权"]
    A --> S["snapshot ID与后续读取<br/>同一原始账本"]
    S --> T["实时授权末检<br/>准备失败也执行"]
    T -->|允许且可交付| D["结果 / 缓冲画布"]
    T -->|错误或拒权| X["不提交画布"]
```

真实整请求分配回归覆盖左右超大头、累计准备、普通／足额成功、初次拒权及末段撤权；TUI要求初始原始预算拒绝后paint未调用、实际后端为空。目标Store3／Engine10／CLI7通过。首次完整构建随后因生产TuiRequest导入误受test条件限制而E0433失败，该编译失败保留，不能算行为RED；唯一修正是无条件显式导入。最终17源清单SHA为`116af7b03bf7d97b152ef96dfe5a7b85b32d652b89cd726bf1ec822ffc11f80c`。

修正源码本机 workspace 已通过 **1401／0／18，60 suites**，限定 fmt、FFI include fmt、严格 all-target workspace Clippy／build、OpenSpec、release CLI／MCP／FFI 及未修改 vendor 124／0／2 通过。单独执行的 **release stdio 18/18、HTTP/SSE 13/13、真实调用的 UniFFI ABI 19/19** 通过；JDK 21 macOS ARM Kotlin 宿主实际通过会话／分页／轮询／v1／release／重开验收，并执行新增 Maven `--errors` 诊断。Kotlin FFI 库 SHA 为 `bb4358e22cf34b025bed1c3c131745c073e5d08120b7f3b700744655ec1e72b5`，协议／ABI 使用 `86fb3c433d050d7ae7067700e96d2b02c7b148a8d5f0e96b44fd4419029b3fe7`；两者来自同一审查源码的不同构建，不能称为同一二进制。已完成的 [D41 回执](benchmarks/query_target_preparation_acceptance_2026_10_04.json)保留 43 份原始／QA／release／Kotlin／原生档案，包含最终本机 workspace 中 20 项目标用例逐案一次通过（14 项新增）的记录；下方原生档案提供独立同源码证据；D43 原生验收见下文；完整任务 13.6 要求复核仍为独立门禁。


`3531943642e5d233f8b95cbd047167bc81899df6` 的 [CI37190906485](https://github.com/loong10k/diskgraph/actions/runs/37190906485) 已终态 **22/22 成功**。四份原始 workspace 日志均逐案确认准备 20 项（其中新增 14 项）及既有查询 32 项恰好一次通过，分组不相加。17 份审查源码摘要与提交逐项相同，43 份 gzip 档案均核验原始长度与摘要。首次 Linux 日志获取遇到本地 gh 缓存 zip 错误，直接 API 获取成功；这是日志收集失败，不是 CI 失败或重新运行。

| 3531943 实际原生 workspace | 通过／失败／ignored | suites |
| --- | --- | ---: |
| Windows stable | 1262／0／16 | 60 |
| Windows Rust1.97 | 1262／0／16 | 60 |
| Linux ARM64 | 1390／0／18 | 60 |
| macOS Intel | 1401／0／18 | 60 |

隔离的整调用分配窗口包含首次授权与目标准备，使用公开发布的合法 2 MiB snapshot ID 和不足以覆盖它的读取预算：

| 公开请求 | 原 Rust requested 累计字节 | 修正后 | 实际修正行为 |
| --- | ---: | ---: | --- |
| 历史 compare／growth／changes，左右任一大头 | 2,102,370–2,131,834 | 4,528–4,541 | 拥有超大 ID 前明确预算拒绝 |
| TUI 导航 | 2,101,248 | 2,521 | 不返回 Layer |
| TUI 整帧 | 2,184,272 | 2,521 | paint 未调用，实际后端保持空白 |

普通及足额请求仍返回真实成功结果。Rust 累计 requested 分配记录成功的 Rust 分配请求，不代表峰值存活内存、SQLite C 分配／缓存、文件系统 I/O 或 RSS，也不是吞吐改进的因果证据。同步 authorizer 与原生 I/O 仍为协作检查，不新增严格内存或墙钟保证，不勾任务 13.6。

## D43 关系、影响、候选与树查询准备：同源码原生验收完成

请求账本从实际 revision 归属准入前开始，必需 snapshot／revision 字段与所有消费者共用剩余额度。Store 提供已准入 evidence reader 与候选窄读，Engine 不再拥有完整 revision 或重复加载目标。错误的 tree scope 在拥有目标前拒绝；初次授权之后的目标／消费者错误及编码阶段错误仍进行实时末检。独立连接在 finish 阶段撤销范围后，权限拒绝优先于该阶段注入的预算错误。

3531943 上先补测试、分别清理并实际重新编译 Engine／Store，取得公开请求 **2 通过／9 失败**与独立 finish 错误 **0/1 RED**。旧累计测试在首条 related 失败后停止，不能说旧七条路径都执行了累计负控。修复后 integration **11/11**、helper **8/8**及固定阶段期限 **9/9**；累计不足／足额配对实际覆盖七条消费者。首次完整 workspace **1411/2/18**以及编译、夹具、构建隔离失败全部归档，不能用最终通过结果替换。

最终冻结源码 workspace **1414/0/18、61 suites**，限定 fmt、FFI include fmt、严格 all-target Clippy／build、OpenSpec、vendor **124/0/2**及 release 构建通过；真实 release stdio **18/18**、HTTP/SSE **13/13**与实际调用 ABI **19/19**通过。14 份上游源码摘要未变；独立 vendor 的 ignored 锁文件仅复制到隔离夹具。独立复审批准最终源码／测试增量。[D43 回执](benchmarks/relation_preparation_acceptance_2026_10_04.json)保留 72 份原始档案，明确区分验收与作废阶段。

合法 2 MiB 目标夹具的整调用 Rust requested 累计分配从 **2,097,770–4,196,125 字节**降至 **348–1,793 字节**，在拥有超大字段前拒绝；不代表 RSS、SQLite C 内存或吞吐测量。`66f2e4c` 同源码 [CI37196289598](https://github.com/loong10k/diskgraph/actions/runs/37196289598) 已终态 **22/22 success**。四份原始 workspace 日志各实际执行 **D43 的 28 项逐案一次通过**（integration11、末检helper8、deadline9）。完整 workspace 为 Windows stable／MSRV 各 **1275/0/16**、Linux ARM64 **1403/0/18**、macOS Intel **1414/0/18**，各61 suites；排除迁移重复运行及清单枚举。12 份审查源码摘要与该提交一致。**13.6 已完成全项源码与实际原生日志复核，全平台仍有 27 项开放**。公开 wire 字段和可信兼容签名保留，内部 helper 接收原始已准入 reader 与剩余额度。

## D42 进程任务：本机基础验收通过，原生执行仍待完成

已实现 ProcessEvidence 持久类型输入、原 metadata/index 授权、fencing、独立 Unix 观测及图库发布／恢复回执；CLI/MCP 将固定 scope/revision/node 路由到同一 Engine。缺少已索引 epoch 或平台资格时先拒绝、零入队。Linux runner 已取得真实 Unsupported 行为 RED，执行器及 15 项阶段回归候选正在验收；编码后目标／根身份、原祖先路径和准备共享预算尚未完成。应用归属及三平台进程能力继续开放。

真实回归复现并修复失败锁存、unwind 句柄计数、授权回调迟到及控制锁准备等待。暂存身份点查增加非唯一表达式索引，保持重复身份拒绝及原 JSON；20k／200k 节点下三次公开写入从360,234／3,600,234条 VM 指令降至各272条。索引建立增加存储与维护成本，此结果不是完整扫描或 RSS 测量。

冻结本机 workspace **1474／0／18，68 suites**。首次 fmt 因一处断言排版失败；仅修正空白后，相关 Git11/11、fmt、严格 Clippy 及 build 通过。macOS 上**未执行** Linux epoch／观测／执行目标，不把零测试算原生通过。[基础回执](benchmarks/process_job_foundation_acceptance_2026_10_04.json)保留96份档案与实测失败。SQLite 外部写锁等待、根祖先绑定、真实原生执行及全平台父项分别继续验收。 首次 f8ed6076 原生 CI 三条 Linux Build 均因夹具调用 Store 私有时钟编译失败，未运行原生行为；现仅改为标准 Unix 秒数，原+60秒期限及断言未变；修正源码0ea5da7的三条Linux原生Build通过，各实际执行epoch2/2、observer2/2，随后两项持久执行在真实scan／holder前置通过后精确Unsupported失败。这是实施执行器所需的行为RED，不是产品原生验收通过。

候选本机 Engine 为 **547／0／8，28 suites**，编译、严格 Clippy 与限定 workspace fmt 通过；Linux 专属阶段用例未在 macOS 执行。D44 原入队期限的 10 项回归先实际 **6／4／0**，补强见证后两项 writer 仍约 1.23 秒返回成功。守卫修复后原 10 项 **10／0／0**，新增提交读锁、实际 SQLite VM 中断见证和 Rust unwind 清理 **3／0／0**，均完整回滚并恢复连接。最终候选本机 workspace **1487／0／18，69 suites**，限定 fmt、严格 Clippy 和 build 通过。Linux 增至 16 项阶段测试（预期 4 项身份／准备负例）及 1 项整调用 Rust 分配观测，真实平台验收仍待完成。这些新增失败及修正验证命令已保留，不以此前基线通过代替当前完成。

同源码 `80d0622` 的 CI37204315054 已取得三条 Linux 原始失败日志：Engine 库各 **392／6／3**。四项身份／准备缺口真实复现；另两项取消／IndexWrite 撤权返回 `StaleOwner`，随后状态断言尚未执行。三条 macOS Rust lane 的入队期限测试也失败：已返回 Budget 且外部锁仍持有，但墙钟为625–961ms，另一次夹具在真正调用前已过期；仍需区分调度／同步 I/O 和实现，不宣称原生通过。Swift 两宿主、Kotlin 五宿主、五种只读原生包均通过；CI现已终止，八条Rust lane均失败、其余十四项成功。Windows stable完整workspace为1348／0／16（69 suites），后续迁移重复测试的unwind准备INSERT被短期限中断；MSRV另有MCP重连响应缺少job_id，旧日志无正文，诊断原因仍待证明。回执保留八条Rust原始日志和完整workspace分阶段统计。MCP仅增强诊断的精确本机测试1／0／0，原身份／终态／超时断言未放宽；首次格式四处布局失败已仅修正空白。

## 以下保留历史阶段记录

后续D31–D40段落保留当时结果、失败和待完成状态；旧任务数或后来已经实现／验收能力的“仍待完成”属于对应历史时点，当前结论以顶部最新阶段为准。失败运行保留，不用后来绿色结果替换。

## Windows 完整原生属性与暂存末检（D31前置，2026-10-04）

schema12 用独立80字节版本记录保存完整128位文件ID、u64卷序号、EOF和原始创建/写入/变更时间。
保留根链约束属性相对打开，每项内核查询前后检查原期限/取消；实时授权与fence最多复用20ms，
批次/发布强制复验。采样不占数据库写锁，末组件只观测reparse对象本身，旧content策略未放宽。
旧行保持未捕获；明确不一致记gap，旧allocated/dedup/目录尺寸或未知身份保持Unverified。

独立审查复现“等待图库锁逾期后仍提交2行staging，随后cleanup隐藏写入”，现暂存/发布fence回调
在等待锁之后末检原时钟及本机取消。回归保留独立写入事件，RED0/1、GREEN1/0覆盖超时与取消。
[验收记录](benchmarks/windows_observation_acceptance_2026_10_04.json)保留原始日志：Core9/9、
Store观测过滤10/10、Engine观测过滤24/24（包含既有用例），最终workspace **1089/0/18（45 suites）**，
严格Clippy/fmt/OpenSpec/release、14vendor摘要、stdio18/18、HTTP/SSE13/13、执行UniFFI19/19通过。
非作者代码APPROVE/架构CLEAR，最终44份源码摘要匹配；Windows新增10个Native和1个Engine
流水线用例须本批同源码CI执行，本机不能替代。

release观测主键窄读在20k/200k未选记录下均18条VM；macOS真实文件负载各4/4，扫描
**0.437/4.391秒**，top/children p50/p95 **6.469/8.564ms**和 **6.773/7.999ms**，数据库+WAL
**43,442,176/436,916,224字节**，直接CLI子进程最大RSS观察 **44,826,624/263,307,264字节**。
相对D30字段增加约2.1%存储；独立观测不证明配对提速、严格RSS或Windows原生规模扫描成本。

上游walk仍按路径；本批根句柄只约束补充采样。真实ReFS高位ID、provider不下载、旧scope编码
来源、可信历史身份利用、原生写、授权采集任务、宿主/mobile和签名部署仍开放；不新增完成勾选。


## 根名称重新绑定与 FFI 源码边界（D32，2026-10-04）

D31 源码 `45d732c9` 原生 CI 终态 20/22：两个 Windows Rust 任务证明属性句柄不必然阻止目录重命名。先补测试的 `5963dd0a` 也为 20/22：注册根被重命名并重新绑定后，旧实现仍返回 `Ok(())`。D31 记录保留两轮失败与原生日志。后续修复从保留父句柄核对当前 drive 和各原名称，比较完整卷/128位ID/创建时间及安全目录状态，允许目录修改时间变化。有限协作复核仍有检查后的竞态窗口，每次复核的原生工作量随根深度增长；新原生 CI 和 Windows 规模成本尚待验证。

FFI 按 API、realm、扫描、JobHandle、共享 JobState 和别名拆分真实实现。两份固定 include 保留旧 UniFFI 词法根；普通 mod 提取初版曾导致19项校验全部变化，修正后真实执行恢复19/19，没有覆盖checksum或包装函数。中文契约使用精确 Rustdoc 条件 `cfg_attr(doc, doc = literal)`。生产 AST 门禁解析真实包含源，并拒绝其他位置的 include；两个遗漏负例实际 RED 后 GREEN，门禁8/8。最终本机workspace **1097/0/18（46 suites）**、严格Clippy、限定fmt和OpenSpec strict通过。同源码原生验收仍待完成，全平台父项不勾选。

release Swift/Kotlin 绑定已重新生成；真实 Swift 宿主编译运行通过会话、分页、轮询、v1及系统 SQLite 共存调用，4份生成Rustdoc页面保留中文参数/返回契约。[D32验收记录](benchmarks/ffi_structure_root_binding_acceptance_2026_10_04.json)保存真实RED/GREEN日志、执行19/19校验及最终源码清单。Kotlin原生运行与Windows行为仍等待新的同源码CI。

首轮D32原生运行 `ca5292b` 的两个Windows Rust构建因测试专用 `path_digest` 重导出未使用而被 `-D warnings` 拒绝，根回归尚未执行。后续仅将该导入条件与唯一Unix测试调用方对齐，保持严格警告及生产行为不变；全部受影响FFI目标54/0、FFI Clippy及全目标workspace构建通过。上述1097/0/18属于前一完整运行，仍须新的Windows原生执行证据。

同一 `ca5292b` 的Windows release包实际通过20k文件原生负载4/4：扫描 **15.738秒**、查询p50/p95 **27.290/42.900ms**、数据库+WAL **54,067,200字节**。此证据覆盖修复后的真实扫描，不替代尚未执行的根单元回归，也不证明配对性能改善；Windows200k和峰值RSS仍未测量。


后续同源码 `c8ff78174e7d7b3f23e8410afc3d16595a48188c` 的 [CI全部22项通过](https://github.com/loong10k/diskgraph/actions/runs/37155851104)。两个Windows Rust版本实际通过全部13个选定的原生观测、根绑定及发布用例，含根与祖先重命名绑定；回执保存终态与Windows stable/MSRV、Linux ARM、macOS Intel日志。Swift/GRDB与Kotlin原生宿主作业也通过。本次完成D32原生验收；命名空间竞态、Windows200k/RSS、provider、原生写入、GUI/移动/真机/签名/生产环境门禁仍未完成。


## 历史尺寸资格与源码组织（D33）

真实旧公共API对未知/读取失败节点及类型替换返回数值增长，未知目录提前Same并暴露占位尺寸。`c8ff781`隔离旧源的正确回归为Core **3/8**、Engine **3/6**、FFI **2/3**（通过/失败）；最初误用根目录的夹具错误另存。Core现统一判断尺寸观察与准确类型，Engine比较每侧独立保留null，只统计有效Size/Contents尺寸变化；已知零值和负增长保留。

查询/比较对象分为17份真实文件，保留公开导出及序列化合同；独立复审确认46份旧函数体中43份未变，仅growth、changes、compare_entry改变。三项AST门禁覆盖本批源码子树并拒绝生产直接path覆盖，不代表整个crate或宏展开认证。

最终本机workspace **1125通过/0失败/18 ignored，48 suites**；新增回归 **11/9/5**、结构门禁 **3/3**、vendor **124/0/2**、14份上游摘要未变。严格Clippy、定向fmt、include显式fmt、构建与release通过；实际UniFFI **19/19**、stdio **18/18**、HTTP/SSE **13/13**通过。真实文件替换为目录的CLI/MCP对照 **3/3**，changes数据一致。非作者代码APPROVE、架构CLEAR，32源清单哈希一致。

| macOS release夹具 | 扫描秒数 | 查询p50/p95毫秒 | 数据库+WAL字节 | 单个CLI子进程最大RSS字节 |
| --- | ---: | ---: | ---: | ---: |
| 20k文件 | 0.673 | 11.970 / 18.079 | 43,442,176 | 44,990,464 |
| 200k文件 | 4.371 | 8.536 / 10.459 | 436,916,224 | 263,421,952 |

各夹具4/4，32次查询、4客户端。时间含CLI启动；RSS由macOS time逐个实际CLI进程测量，不是并发总RSS。非配对观察不能证明提速、严格RSS或历史查询吞吐；Windows200k/RSS仍未测。原始失败、成功命令、协议、测量脚本及摘要见[D33回执](benchmarks/historical_size_acceptance_2026_10_04.json)。

同源码 `ca7813cba28d2abdef8309d2d3176893559d68c5` 的 [CI已22/22通过](https://github.com/loong10k/diskgraph/actions/runs/37158622289)。两个Windows Rust版本、Linux ARM及macOS Intel日志各实际执行全部28个选定D33回归（Core11/Engine9/FFI5/结构3），回执核对32源摘要与该提交一致。本次完成D33增量原生验收，当时Q04任务3.9因实际scope兼容性（D34）继续开放；Windows历史身份及历史正文绑定仍为独立未完成要求；本批不完成任何全平台父项，CLI/MCP危险写工具保持关闭。

## 实际历史命名空间（D34）

两侧均获授权不代表命名空间相同；合法已归属旧v1记录可具有不同无损根和相同显示字符串。正确回归记录为Engine **7/2**、隔离旧源扩展 **9/2**、FFI **5/1**（通过/失败）；初次APFS建目录错误与FFI标量断言错误明确不计缺陷证据。

Engine与旧FFI增长复用已有reader核对持久实际owner/server/scope，不同scope返回null增长，或保留 `different_root` 变化标签并新增 `scope_changed: true`。注册API保证scope根不可变；可信资格helper不授予权限，原响应预算与终态检查保留，包括Engine编码后复检。通用授权跨根比较和同scope已知旧历史仍可用；没有新增schema、依赖、owner、线程或修改导出。

最终源码本机workspace **1142通过/0失败/18 ignored，49 suites**。Engine命名空间 **11/11**，FFI新增 **6/6**、全部受影响增长用例 **20/20**；严格Clippy、定向fmt、include fmt、OpenSpec、构建和release通过，实际UniFFI **19/19**、stdio **18/18**、HTTP/SSE **13/13**。独立代码APPROVE、架构CLEAR，最终20源摘要一致；原始失败、成功日志及源码摘要见[D34回执](benchmarks/historical_namespace_acceptance_2026_10_04.json)。

Unix离线夹具以自然非法字节产生显示碰撞，Windows启用夹具以本平台自然未配对UTF16产生碰撞；均为公开API注册的合法合成元数据，不声称建过非法目录或执行过真实迁移。Linux另有2项真实原始目录用例，本机macOS未执行；现有socket验收运行过，但没有新增socket显示碰撞夹具。

同源码 `407125f62fda994826a7858737b22fa95efe4cb4` 的[原生CI已22/22通过](https://github.com/loong10k/diskgraph/actions/runs/37161135994)。两个Windows Rust版本和macOS Intel各实际执行全部 **17** 个选定用例（Engine11/FFI6）；Linux ARM另执行 **2** 项真实文件系统用例，共 **19** 项。20项已审源码摘要均与该提交一致；[D34回执](benchmarks/historical_namespace_acceptance_2026_10_04.json)保留终态元数据、原始job日志、实际执行用例名称和此前失败观察。

Q04任务3.9的完整C06/C07矩阵在下方D35继续，新源码仍需原生验收。D34完成命名空间增量原生验收，不完成全平台父项；命名空间相同不证明历史文件身份连续或正文版本相同。

## 原生输入准入与完整历史语义（D35，2026-10-04）

Git元数据和stash日志已知超限时先拒绝正文读取；每块同时受初始未读长度和剩余共享额度限制，终态核验长度及既有强身份/版本。reflog保留原OutputLimit分类和首次失败锁存；精确额度与空文件有效，管道EOF和清理契约未改。真实句柄偏移复现原4096字节及零长度超读，Git目标从4通过/3失败到8/8。

ScanCoverage以独立真实对象文件统一覆盖判定，字段、serde及公开导出不变。Core增长/变化/候选和Engine历史不能以complete标志覆盖不可读或深度缺口；Store原先就拒绝矛盾发布且继续整体回滚。Core真实RED为5通过/3失败，随后8/8；Engine10项涵盖六种设置双向变化、正负/零增长、未知/类型替换、卷/provider域值、partial、无重命名推断及真实宿主扫描增长/重命名。合成导入与opaque URI语义不证明挂载换卷或provider操作。

最终本机workspace **1168通过/0失败/18 ignored，51 suites**；构建、严格Clippy、fmt、OpenSpec及release通过。实际UniFFI **19/19**、stdio **18/18**、HTTP/SSE **13/13**及14项上游摘要通过。独立代码APPROVE、架构CLEAR与最终18源清单一致。[D35回执](benchmarks/native_input_history_matrix_acceptance_2026_10_04.json)保留两组真实RED、初次错误分类/Clippy失败及新鲜最终日志，不声称配对吞吐或RSS提升。

任务3.9已在2450ab1同源码原生CI37164196395终态22/22及四份日志各26项实际成功验收；Git8.7/15.13仍需授权私有工作树捕获、持久typed输入/能力上限、内容权限fence、证据发布及CLI/MCP验收。现有OpenSpec设计仅记录下一实施契约，未启用新collector。原生写、真实provider、GUI/GRDB/Room、mobile/设备、签名和生产父项保持开放。


任务状态复核：167项中137项已完成、30项开放（包含汇总父项，不等于30个独立漏洞）。8.6重新打开：可信库进程采样不等于已授权产品collector；尚缺应用安装实例采集、PID启动及资源身份绑定、来源持久化与原子revision发布的端到端验收。已验收的解析、预算和通用发布子能力保持完成。

## 受约束捕获与查询准备（D36）

Git 可信 scope 入口采用保留的原生根／组件句柄和独占捕获工作树，拒绝 scope 外依赖、链接、源修改和根路径链替换。私有捕获保留固定 status 命令的执行权限语义，任意命令不属于该契约。Windows 实现仍待本增量原生运行，真实 provider 不下载尚未验收；授权持久任务、能力上限和原子证据发布仍待实现。

TUI 原始准入采用所需字段投影、整帧累计读取预算和无 payload 续页探针。首末控制锁争用及时拒绝，交付须遵守原期限、实际归属和实时授权。入口从1182行减为43行，真实对象和逻辑分别落在模块文件中。候选准备也将快照覆盖头计入共享原始字节预算；到期且未观测头时返回类型化空 Deadline 与 `coverage_observed: false`，字节不足和真实存储故障仍是错误。新增公开 Rust 字段要求下游字面量补字段，旧 wire 字段与 UniFFI 签名保留。

首轮全 workspace 为1227通过、2失败、18 ignored，两处失败完整保留：非标准测试模块路径未通过源码门禁，覆盖头有界准备最初改变了旧到期候选结果合同。标准模块位置随后通过3项布局和5项文件读取回归，期限合同修复通过 Store 候选10项与 Engine 请求预算8项。最终源码 workspace **1233/0/18（51 suites）**、全部七项质量门禁与 release 构建通过，真实 release stdio **18/18**、HTTP/SSE **13/13**及 UniFFI 校验值 **19/19**通过；原生 CI 尚待完成。[Git](benchmarks/scoped_git_capture_acceptance_2026_10_04.json)与 [TUI／查询](benchmarks/tui_budget_acceptance_2026_10_04.json)回执分别保留真实阶段证据，本处不关闭平台父项。

Release 20k／200k 文件各4项检查通过，32次查询、4客户端；扫描0.503／5.169秒，查询p50/p95为9.048／11.323及8.492／10.921毫秒，数据库+WAL为43,442,176／436,916,224字节，单个CLI子进程最大RSS为44,433,408／263,274,496字节。RSS由macOS time逐进程测量，不是并发总量；非配对观察不证明提速或严格内存上限。最终源码清单 `67cbee45…` 包含等价私有导入拼写，以及保留旧图例导出路径的一处明确局部expect；两次严格Clippy失败均已归档。

D36 原生运行已结束：`e32214d` 的 [CI37171948796](https://github.com/loong10k/diskgraph/actions/runs/37171948796) 为20/22，两套Windows工具链的相同两项范围边界回归失败；Linux ARM64与macOS Intel实际通过64/64与65/65选定用例。[原生观察回执](benchmarks/d36_native_observation_2026_10_04.json)保留终态及原始日志；先前“待完成”描述属于当时状态。诊断统一及原生禁止替换/owner释放正控已修改，但新源码尚未通过Windows原生验收。D37真实socket回归复现远程Index/Sync任务在原始token到期后仍发布revision，修复进行中，不能声明生产就绪。


## 持久请求授权与认领预算（D37）

Index/Sync任务持久保存不可变请求来源、token能力上限和原始绝对到期时间，在入队/合并、严格认领、执行、staging及图库最后SQL后的commit前复验，续租不延长原始token。远程旧未知来源任务安全拒绝，可信本地兼容保留。控制库v6→v7升级使用一致性备份、原始字节准入及不可变授权记录；损坏来源返回InvalidGraph并停止本轮，需要管理员修复，不声称自动隔离继续处理。

真实socket排队后token到期、runner候选页及认领后准备计时均有实际RED→GREEN。原50ms预算任务在认领后持图库锁等待100ms仍成功发布；修复后同一测试返回BudgetExceeded/Failed，无staging、snapshot、revision或collector发布。原生协作取消仍可能短暂超时，不承诺硬I/O墙钟、RSS或跨库原子性。

开发工作区为**1273通过/7失败/18 ignored，54 suites**，7项全部属于尚未实现的CLI/MCP Git产品入口合同。精确暂存安全候选导出时未包含两个从未tracked的未来功能测试文件，**1270/0/18，52 suites**；原有测试和新增授权回归全部执行。未来测试原文保留在工作区，失败日志归档，不据此验收Git8.7/15.13。候选构建、严格Clippy、限定crate的fmt、FFI独立include格式、OpenSpec strict及release CLI/MCP/FFI通过；实际release stdio18/18、认证HTTP/SSE13/13、实际调用UniFFI公开校验值19/19通过。非作者复审确认34份安全来源及另2份Windows纠正，摘要全部一致。

[D37回执](benchmarks/durable_job_authority_acceptance_2026_10_04.json)保存两种候选范围、原始日志、摘要及边界。Release20k/200k各4/4检查通过；扫描0.477/4.396秒，查询p50/p95为7.001/9.744及6.925/8.676毫秒，数据库+WAL43,458,560/436,932,608字节。非配对观察，不声明提速，本增量未测RSS。修改后原生CI仍待完成，15.20及30项未完成实现/验收/父项继续开放。本机存在Android平台/构建工具，但无NDK和连接设备；仅macOS命令行工具，无iOS SDK/模拟器。这些环境观察不能替代设备验收。


## D37 原生结果与 Windows Clippy 纠正

`b680a1ce35a118d1b5c396c046eaa9a26193978f` 的 [CI37176087169](https://github.com/loong10k/diskgraph/actions/runs/37176087169) 已终态 **21/22**。Windows stable/1.97、Linux ARM、macOS Intel 的选定 workspace 阶段均实际通过39项授权/范围边界回归；对应总数为1131/0/16、1131/0/16、1259/0/18、1270/0/18。Windows stable 在测试通过后因三处严格 Clippy 检查失败，不能算完整流水线通过。D37回执已保存终态、四份原生日志及逐项执行清单。

后续仅改 Windows 尾表达式和两处等价整除判断；精确暂存候选的本机fmt、13项范围回归、workspace严格Clippy及all-target构建通过，新源码原生CI仍待。未来Git产品代码和CLI拆分尚未暂存或验收；15.20仍包含collector授权门禁，不关闭任何父项。


D38 的同源码 `fb7757c3727ee9d2839b2b1a7a404d2a1f924295` [CI37177944062](https://github.com/loong10k/diskgraph/actions/runs/37177944062) 已终态 **22/22 success**，Windows stable 的严格 Clippy 同样通过。Windows 双 Rust、Linux ARM、macOS Intel 四份原始 workspace 日志各再次确认39项授权/范围边界回归实际通过；D37失败记录仍保留，D38终态、原生日志及源码摘要已追加到同一回执。此验收完成授权基础与等价 Clippy 纠正，不验收工作区中尚未完成的 Git collector 产品闭环，也不关闭15.20或全平台父项。


## 持久 Git 采集与 CLI 文件组织（D39）

CLI/MCP 显式 Git 入口持久保存原请求授权与固定索引目标，在预算内捕获私有输入，并在同一图库事务发布revision、来源选择和唯一任务回执。恢复只核对已提交回执，不重新采样。基线按实际server/scope判断；损坏来源引用明确拒绝。取消标志仅属于本机运行代次。CLI状态data保留真实ID和既有外层字段；MCP状态envelope与data使用同一授权投影。

完整固定候选包含此前未tracked的产品测试：**1378通过/0失败/18 ignored，58个报告suite**。134份独立审查来源摘要均匹配。限定crate的fmt、FFI独立include格式、workspace all-target严格Clippy/build、OpenSpec strict、未修改上游扫描器124/0/2和release CLI/MCP/FFI构建通过。实际release stdio18/18、认证HTTP/SSE13/13、调用UniFFI校验值19/19通过。真实RED与三处测试夹具纠正分别保存在[D39回执](benchmarks/git_evidence_job_acceptance_2026_10_04.json)；[159项本机清单](benchmarks/git_evidence_job_native_cases_2026_10_04.json)记录逐项实际日志，不代表其他平台已验收。

同一release负载脚本前后在20k/200k文件均4/4通过。扫描0.546→0.470秒、4.654→4.699秒；查询p50/p95为8.433/10.243→8.387/9.612毫秒及8.627/9.791→8.829/10.261毫秒。单CLI子进程峰值RSS为45,154,304→45,023,232字节及263,831,552→263,766,016字节；数据库+WAL两档均增加36,864字节。单次顺序配对测的是普通扫描/查询，不证明因果提速、Git吞吐或严格RSS。CLI main为82行，18辅助函数/33命令分支保留原行为；10个旧适配模块仍未纳入新增结构门禁。

D39的`bac84f72ba44b3398c15de7ef19d0717f4653820` [CI37184551143](https://github.com/loong10k/diskgraph/actions/runs/37184551143)已终态**21/22 success**。Windows Rust1.97.0的原始到期测试未到达采集后发布回调，原日志未记录提前返回的错误。四份workspace结果为Windows stable1239/0/16、MSRV1238/1/16、Linux ARM1367/0/18、macOS Intel1378/0/18。159个选定本机用例中Windows实际执行157项（MSRV156通过/1失败），另2项仅Unix用例未运行；ARM与Intel各159/159通过。D39回执保存终态、四份原始日志及逐项观察，失败流水线不能当作全绿验收。

D40隔离诊断在原3秒exp不变时加入仅测试的4秒采集前延迟，真实持久门禁返回`Conflict("job request authority denied")`、任务Failed且没有进入发布。这证明一种相同失败机制，不证明原Windows宿主所有可能提前错误的唯一原因。修正夹具在入队前按默认15秒执行预算加5秒准备余量固定原exp，真正捕获后核验原authority和Running状态，在25秒有界窗口等待真实到期，再断言精确持久授权拒绝、身份不变、分类错误、无发布和无取消句柄。没有续期token或修改生产限额。该等待也超过运行预算，精确授权断言不算独立期限优先级或性能验收；修正后的原生CI仍须通过。

本轮结果不关闭父项；全平台目标、默认关闭的写工具、provider/宿主/移动端及发布门禁继续开放。

## MCP 服务组织与最终本机验证（D40）

RT-10已实现55行入口，服务/配置/分发/身份/范围/工具/stdio承载真实职责，原服务测试实际挂载。两位非作者独立复核24份来源清单及67个函数正文一致；36个既有非入口来源逐字节未变，21个迁移测试均在新限定路径实际通过。原auth/http/protocol大文件仍不在本增量门禁范围。

结构TDD先对原入口记录5/1 RED，再对真实直接/嵌套条件路径替换记录7/2 RED；修复后9/9通过，包含畸形条件拒绝与真正挂载的安全属性正控。到期执行套件9/9通过，同一4秒准备延迟配对控制由0/1转为1/1，仍要求原身份不变和精确持久拒绝。

最终固定候选为**workspace1387/0/18、59 suites**，限定fmt、FFI独立include格式、workspace all-target严格Clippy/build、OpenSpec strict、上游124/0/2、release CLI/MCP/FFI构建通过；14份上游来源摘要一致。实际release stdio18/18、认证HTTP/SSE13/13、调用UniFFI校验值19/19通过，最终release摘要与这些实际执行二进制一致。本次源码组织不改扫描/查询算法，不声明新的性能或RSS结果。[D40回执](benchmarks/mcp_service_layout_acceptance_2026_10_04.json)保存真实阶段、源码摘要、失败及边界。

**D40原生CI之前的历史时点：**当时修正源码原生CI仍待完成，清单为167项／137完成／30开放。顶部最新阶段记录后续同源码验收及140／27状态；全平台生产就绪尚未成立。
