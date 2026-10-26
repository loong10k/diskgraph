# 当前三平台只读CLI/MCP验收状态

## Windows 桌面机实测：658bade（2026-10-09）

仍未生产就绪。用户提供的 Windows 桌面机仓库 `E:\workspaces\workspace-loong10k\diskgraph` 已快进到 `658bade82738562e2c882042b3203bc14fdc2be3`，原 10 项覆盖源码与该提交逐字一致后保存到独立 stash，不丢弃此前 stash 或用户改动。最新 GitHub CI 为 37865556195，本段写入时仍运行，不能当作全平台通过。

原生默认测试并发、真实 worker 与当前源码 driver 的结果：Engine 670 通过、53 失败、7 忽略；CLI 82 通过、6 失败；单独 MCP 229 通过、2 失败。CLI 失败使原组合测试未到达 MCP，故另行执行 MCP，不能重复累计。三轮均保留原期限，源码前后摘要相同；失败记录见 `docs/benchmarks/windows_{engine_full,cli,mcp}_658_2026_10_09.json`。Git 多项期限/清理恢复失败，以及查询在进入目标断言前期限耗尽仍待定位；数量为测试断言失败数，非独立缺陷数量。

另外发现旧容量资格回放脚本对新增诊断模块依赖适配不完整，导致 Windows CI 在 E0432 编译错误提前终止。该失败与上述真实默认并发产品回归分别记录；脚本合同测试修复通过不等于产品回归通过。原生脚本资格结果与当前候选状态以 `windows_capacity_replay_diagnostics.md` 为准。200k 完整期限、当前提交所有原生平台 CI 和既有生产父门禁继续开放。

## 最新核验：a6eab41（2026-10-08）

**尚未生产就绪。** 当前交付范围为 macOS/Linux/Windows 只读 CLI 与 MCP；危险写工具仍关闭。当前提交为 `a6eab41ce8ab1744e5a7e8144aa14c97cdb6fe63`，当前 CI 为 `37714194463`。本次实时查询 Windows/Linux x86_64 Kotlin 宿主已成功，其余16个已列出的job仍排队；不是完整平台通过。下方历史提交的通过不能合并为当前同 SHA 全平台通过。

| 门禁 | 最新事实与剩余差距 |
|---|---|
| Windows 验收进程树回收 | `43c82d4` 原生 8/8，无跳过；实际200k外层超时后原异常传播，Job退休和管道收集未报告未知状态。仅验证验收工具，不能替代产品guardian的OS身份隔离。 |
| Windows 200k 性能 | 串行准备两轮170.38/207.87秒，随后在index阶段外层300秒超时；没有证明扫描本身超过300秒。`04d28b1`四线程原生准备235.664秒，index仅剩约63.804秒，仍外层300秒超时；不能宣称Windows准备优化成功，也未证明扫描本身超过300秒。 |
| Linux 不触发数据下载 | `249f6e5` ARM64真实FUSE直接/嵌套read/hash 4/4；普通读正控恰一次fetch，原provider卸载/回收均核验。只覆盖这个provider夹具，通用Linux文件系统资格仍缺。 |
| macOS 云文件 | 线程禁止物化机制已有实现；真实FileProvider下载计数与普通读正控缺失，已请求专用验收环境。 |
| 查询期限与历史一致性 | 发送绝对期限/generation/expiry保护已有代码和历史真实socket证据；资源紧张的间歇失败根因、Windows目录时间场景仍需收敛，不能用后来单轮绿色抹去。 |
| 产品监督恢复 | guardian/原用户凭据executor的职责、原生退休与容量闭环仍开放；验收Job修复不关闭此项。 |
| 最终平台及性能收口 | `04d28b1` Linux ARM64/x86_64包测试均因旧mock入口错误终止，未进入后续真实包验收；`249f6e5`修正为mock实际run_acceptance，本机7/7。Windows旧轮真实包验收已因200k外层超时失败；当前a6eab41已有Windows/Linux x86_64 Kotlin宿主成功，Rust完整矩阵仍排队。历史d971427 Linux配对性能在200k baseline退出101、Business(Conflict)，候选200k和deep未执行；不能算候选失败或完整性能比较。仍缺最新同SHA全部目标、打包RSS/长期运行与完整性能对照、最终验收。既有Linux采样RSS是历史独立实验，不能冒称当前全平台证据。 |

证据：`docs/benchmarks/windows_native_job_43c82d4_2026_10_08/`、`windows_200k_stage_8ccdb41_2026_10_08/`、`linux_fuse_native_04d28b1_2026_10_08/`。正式tasks仍有27项开放，含父项和所选范围之外任务；此表不是27项逐条完成审计，未勾选任何父项。

本批新增：HTTP正文非法UTF-8明确拒绝，Content-Length仅接受非空ASCII十进制并拒绝溢出；真实parser/socket窄范围7/7，双路独立审查通过，完整MCP原生CI待验证。验收脚本全量276项：275通过、1项Windows真实Job测试因本机平台跳过；不替代Rust workspace及原生目标门禁。证据见 `docs/benchmarks/http_content_length_decimal_2026_10_08/`、`linux_package_mock_boundary_2026_10_08/`、`python_acceptance_regression_249f6e5_2026_10_08/`。

## 当前剩余事项计数边界

选定只读CLI/MCP范围按8类跟踪：产品监督与隔离、云占位保护验收、查询期限稳定性、Windows文件身份、应用/进程归属产品接线、Windows200k负载、性能/RSS/长期运行、最新同SHA全平台CI。8类不是独立漏洞数量；正式27个未完成任务包括范围外任务和聚合父项，未勾选任何任务。

本批代码已交付：56a1045占位文件digest末段授权/取消/期限复验（目标17/17）；f784fd0迁移备份独占新路径及原备份保留（迁移21/21）；a6eab41准入等待后原期限复验、拒绝数据库出生（host9/9）。目标检查与独立审查通过不替代全平台验收。Store当前工作区319通过/5忽略含未提交reader_observation测试，不能冒称当前commit精确测试清单。CLI完整本机71通过/12失败，12项在原生部署初始化Unsupported处失败，未进入业务断言。

Linux历史基线诊断边界：2a2f828原scan_jobs keeper丢弃原检查错误，旧scan_execution也可能折叠为Conflict；LinuxScanRoot namespace复验另有Conflict路径。失败后租约/权限正常的非原子快照不能排除现场失败。当前job_execution已保存keeper原错误，scan_execution对check_now原样传播；尚无证据证明历史失败根因，禁止放宽预算或替换基线冒称收敛。原始证据见docs/benchmarks/linux_scan_cost_baseline_conflict_d971_2026_10_08/。

## 历史核验记录（按原提交保留）

源码：b03de97b32e767c23b04e8308b876cd9eaee4a88；CI：37683012612。状态不是生产就绪，不勾选整体任务。

已关闭本批缺陷：Unix staging固定1024计费改为真实载荷加16，扫描累计nodes/staged_bytes溢出明确停止。新真实socket并发身份隔离及实际SQL载荷一致性测试已合入。

当前同SHA Linux x86_64与ARM64原生包分别stdio18、HTTP13、升级回滚7、受控负载4，全组通过；原日志/摘要见docs/benchmarks/ci_37683012612_linux_{arm,x86}_package。Windows x86_64原生包同样42项通过，原日志见docs/benchmarks/ci_37683012612_windows_package；Windows完整原生测试仍在运行，macOS ARM/Intel排队，不用旧SHA或本机局部测试代替。

仍需关闭的五类门禁（不是精确缺陷数量）：

1. 监督恢复产品接线：guardian/原用户OS凭据executor职责调整仍待确认；生命周期/IPC/容量槽/真实退休闭环未完成，不通过拒绝全部配置假装实现。
2. 资源紧张时查询期限稳定性：CPU配额失败仍开放；保持授权期限和新鲜归属，不未经验证改用预开连接。
3. 云占位文件不下载：缺三桌面真实provider验收。
4. 三平台同SHA最终验收：Linux ARM64 stable完整CI通过（含314/0/5串行Store门禁）；Linux x86_64 Rust1.97完整CI通过（Engine历史兼容10/0，串行Store314/0/5）；Linux x86 stable完整CI已通过，Windows完整矩阵仍运行中，macOS排队；Linux包成功不证明这些门禁。
5. 性能及交付收口：最新八轮配对功能通过，200k扫描/查询结果方向反转；无既定SLO/RSS对比/其他原生平台性能。新小夹具关系阶段100请求通过，不能解释200k CLI波动。最终全检查/文档/证据收口需完成。

范围外GUI、移动端、FFI和写操作任务继续保留未完成；危险工具不因库测试通过而启用。

新增传输审计：SSE控制锁等待与debug UTF-8 panic/正文泄露已有候选修复，Linux/macOS真实SSE争用3/3，Linuxdebug真实进程3/3。仍未完成的三个具体问题：现代响应分段发送绝对期限及授权generation复验；现代SSE噪音放大；响应后重认证造成日志主体漂移。候选不代替同提交三平台门禁。

后续候选关闭现代SSE两输入来源噪音放大和响应后日志重认证主体漂移，真实认证socket均有RED/green与双路审查。当前具体传输代码缺口仅剩现代响应发送绝对期限及发送期间授权generation/expiry复验；五类产品门禁仍保持开放。

状态纠正：原生 Windows MSRV CI 37690643575/job113029625657 的真实 CFAPI provider 内容测试 1/1 通过，产品 fetches=0 和普通读 fetches>0 正控标记均执行。e8333b3 至 fa400fa 内容模块及 provider 夹具无变更；证据见 docs/benchmarks/windows_cloud_e833_revalidated_2026_10_08。不能称三平台 provider 均缺失；剩余 macOS/Linux provider 和最新同SHA平台闭环仍开放。整个旧Windows job 因另一个授权预算测试失败，不投影成Windows全量成功。

Store 入口大文件复核：当前 lib.rs 339 行，仅模块声明/重导出；本机 ARM macOS AST 结构合同测试 1/1 通过。保留既有两行 cfg(test) reader_observation_tests 修改，未混入提交。此项关闭原入口集中实现问题，不代表整体优秀架构或运行时验收。证据见 docs/benchmarks/store_layout_revalidated_2026_10_08。最新提交 fa400fa5e6119a85b9fad74ccfa8e18bdd56accb，CI37695471590仍运行，Windows MSRV root-parent原生步骤正在执行，macOS仍排队。

当前 fa400fa 的独立 Git archive release 构建已在 Linux ARM64 Docker 完成20k/200k宽目录负载，每组32查询/4客户、功能4/4。20ms采样进程树RSS与前后产物摘要见 docs/benchmarks/release_rss_fa400fa_2026_10_08。单轮、采样有扰动，不能称严格内存上限、性能改善或跨平台SLO达标；性能门禁仍开放。

同形新夹具 AB/BA 与20ms进程树采样RSS对照已完成8轮，5ca2d40旧版与fa400fa当前二进制前后摘要一致，功能32/32通过。200k当前RSS采样高于旧版，不能宣称性能/内存优化完成；扫描与查询差值需结合原始结果评估。证据见 docs/benchmarks/paired_rss_fa400fa_2026_10_08。单个Linux ARM64环境、采样干扰及无既定SLO边界保持。

RSS贡献诊断新增一对200k负载，功能8/8通过。记录采样峰值处各原子进程贡献，原始数据见 docs/benchmarks/rss_attribution_fa400fa_2026_10_08。只定位采样峰值贡献，未获取堆分配栈或精确业务阶段，不能直接归因于某个对象或算法，也不消除前述回归观察。

最新fa400fa Linux stable job113046019929失败：Engine652/1/9，display_complete_reader_refuses_token_expired_during_consumer未进入消费者。增加实际错误诊断，原验收不变；隔离CPU2/10线程admin policy组20/20不能覆盖CI失败。日志见 docs/benchmarks/display_consumer_ci_failure_2026_10_08，问题保持开放。Windows MSRV正在Test步骤，尚无最终结果。

当前release深目录验收补齐Linux ARM64单轮：300层目录、300文件，数据库精确601节点/最大深度301；32查询4客户、revision绑定和结果上限通过。扫描0.120秒，p50 12.987ms/p95 17.760ms，数据库及WAL3100672字节。产物仍绑定fa400fa，后续6fbf3ee仅改测试诊断，不把旧产物重标为新SHA。见 docs/benchmarks/deep_release_fa400fa_2026_10_08。无深目录RSS/配对或其他原生平台，不勾选整体生产门禁。

最新核验补充（2026-10-08，保留上述历史状态）：Linux c71a855 的 ARM64 stable、x86_64 stable/1.97.0 三个完整原生 Rust job 均成功；Windows 6fbf3ee 的 stable/1.97.0 两个完整原生 Rust job 均成功。分别以 linux_c71_full_ci_2026_10_08/receipt.json 和 windows_workspace_6fb_revalidated_2026_10_08/{stable,msrv}_terminal.json 为终态证据；Windows 目录早期 receipt.json 的 in_progress 记录是历史观察，不再代表终态。不同提交的通过不能合并为最新同 SHA 验收。

最新 HEAD e3ccbf4 已与 origin/main 核对一致。负载验收新增按实际 revision 对应 snapshot 核对预期节点数的门禁，2/2 回归通过；原生包验收新增 200k 文件负载。CI 37702662951 本次查询的 18 个 job 全部排队，尚无最新原生运行结果。节点数量门禁不证明逐路径完整性；性能和平台门禁保持开放。

传输状态纠正（当前源码 aa9819e 核对，保留上述历史记录）：http_delivery_authority 已绑定原请求到期与分发前授权 generation，http_delivery_writer 逐块使用原绝对期限、发送前/授权后的期限检查，并在真实发送前复验 generation 与到期。9758b53 的 Windows MSRV 整个 job113071996228 通过；原始日志中 9 项 http_delivery_writer_tests 以及 actual_remote_nonreading_peer/slow_progress 两项实际 socket 门禁执行通过。因此不能继续将该发送保护记为“无实现”；当前提交的同 SHA 三平台验收仍缺失，不勾选整体生产任务。

物理负载门禁补充：2cd5860 Linux x86_64/ARM64 原生包实际两档各6/6；20k与200k存储节点分别精确20001/200001，同时完整路径集合、父子关系、4客户结果绑定和节点超限不发布均通过。独立下载校验记录见 docs/benchmarks/linux_{x86,arm}_package_2cd5860_2026_10_08。主分支后续报告与夹具生命周期修复不将这些历史产物重新标为当前提交；无packaged RSS、SLO、长时间及其他平台证明。

最新Windows并行夹具失败证据：`docs/benchmarks/windows_parallel_fixture_04d28b1_2026_10_08/`。保留原300秒门禁，后续须分清夹具准备成本和实际产品扫描成本，不能以减少文件数或放宽期限获得通过。
