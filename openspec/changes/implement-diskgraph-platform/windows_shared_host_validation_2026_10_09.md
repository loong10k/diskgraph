# Windows 台式机共享负载验收记录

Windows 工作区 `E:\workspaces\workspace-loong10k\diskgraph` 已正常 fast-forward 到 `2552d0823f9a72a4130e56914230406e08d30a0d`；验证过的覆盖文件先与该提交逐字比较，再仅对这些路径保存 stash `978f1d1a38c22d4555c129b06591c84d185a7a13`。跟踪文件干净，原有两个 Python 缓存目录保留。没有 reset、切分支、删除旧 stash 或改写上游扫描器。

原生定向结果已经保存为 `windows_staging_revocation_settlement_2026_10_09.json`。随后启动全 workspace、all-targets、no-fail-fast、默认 Rust 测试并发的原生验证，Job `wc_job_ri-nnH2baAP0iL7q`，输出目录 `C:\Users\hnxyh\AppData\Local\Temp\diskgraph-desktop-jetfdyj0\windows_workspace_2552d082_oct09`。原 worker/driver 镜像绑定保留，未设置 RUST_TEST_THREADS；本次整包执行期限独立于原 Windows 200k/300 秒性能门禁，不能替代后者。

运行中观察到多个 CLI 编码终态撤权测试在实际 hook 之前返回 BudgetExceeded，以及 Engine `revision_reader_initial_callback_can_reenter_control` 的未撤权成功分支超预算。它们未到达目标安全断言，不能记作撤权覆盖通过，也不能仅凭这些错误声称发生数据泄露。原 Job 已终态失败，退出码101，实际测试执行1431.468秒。完整证据见 `docs/benchmarks/windows_workspace_2552d082_2026_10_09.json`；源码前后45个文件摘要一致，HEAD不变，跟踪文件仍干净。

资源诊断 Job `wc_job_TbbgQ4lv0qyhs--n` 的实际快照：24 逻辑处理器、33289300 KiB 总物理内存、1137364 KiB 可用。三个较早启动的 link 进程工作集分别为 2563588096、2571923456、4136050688 字节。随后 Job `wc_job_Z2XQ07bBE7k4fiJ_` 核实它们经 rustc 分别归属 `F:/codex_build_cache/druid_rust/task7_full215_target`、`task7_standard_cargo13_1971`、`task6_lifetime_clean_1971_20261008`，并非本次 DiskGraph 验证。没有结束这些其他项目的进程。

共享内存压力是待控制的环境因素，不是已证实的全部根因。原失败必须保留；不得通过放宽请求期限、隐藏测试、设为串行或杀掉其他工作来获得通过。后续需在资源条件可比时验证默认并发及 release 性能，并独立完成 CI、200k 性能和有限前端退出门禁。

同 SHA CI `37880305608` 的格式 job 发现两个新增模块声明排序问题，打包因依赖格式检查而跳过；这是本轮交付检查遗漏，不能记为打包成功。修正只移动两条声明，提交 `f5355bd9`；项目 CI 原格式命令已在本机实际通过。`cargo fmt --all` 会包含禁止修改的 vendored crate，不能据其输出批量格式化上游源码。上游 pin/摘要集成测试4/0。

全量失败目标为 CLI bin、Engine lib及 growth_narrow_read / hardening / history_compatibility_matrix / relation_preparation_budget、FFI lib、MCP lib，共8个。对应通过/失败/忽略分别为75/13/0、643/87/7、4/1/0、14/1/0、5/5/0、8/3/0、89/4/0、214/19/0。原日志记录133条失败测试行，不等于133个独立缺陷。Store主测试382/0/8，保留此前独立运行380/2/8的期限失败，不能以本次通过抹去。

诊断包装器的 failed_tests 正则误加转义，最初得到空数组；已从原 stdout 按实际行前后缀重新提取133条，并保留修正说明。测试结果及原始日志摘要没有改变。FFI 的两项 listing 失败分别返回 budget_exceeded 和 Store BudgetExceeded；后者错误文本为“query response budget exceeded by one record”，单凭该文本不能判定耗尽的是字节预算，须继续隔离诊断。未观察到这些失败返回成功数据，仍不能据此代替安全断言通过。

后续单独诊断 Job `wc_job_cgu34JEt_wRh2lTI` 终态0，`diskgraph-ffi --lib native_listing_tests` 9/0，测试阶段22.33秒（含依赖重新编译的命令255.375秒），保持默认并发与原期限。证据见 `docs/benchmarks/windows_listing_diagnostic_2552d082_2026_10_09.json`。单包与全 workspace 的 Cargo feature unification 不同，worker/driver实际摘要独立保存；此结果不能替代全量通过，也不能证明共享负载是唯一根因。

进一步原程序路径复用诊断 Job `wc_job_FCQ2l4gEjgSrxh0e`：不重新编译，从原workspace日志命名路径复制并冻结FFI测试程序，9项listing默认并发通过；同一冻结程序随后完整FFI默认并发87通过/6失败。失败集合与原89/4不同，均保留实际错误；不能以子集绿色关闭全量问题。原全量运行未单独记录FFI程序摘要，因此只证明本次两次运行之间字节一致，不能追溯声明与更早全量程序逐字一致。源码前后45项摘要一致，worker/driver仍使用原冻结镜像；证据见 `docs/benchmarks/windows_workspace_ffi_reuse_2552d082_2026_10_09.json`。

同SHA GitHub Windows stable job `113658242776` 已终态success，完整原日志及元数据保存于 `docs/benchmarks/ci_2552d082_passed_targets_2026_10_09/windows_stable_*`。这是独立CI环境的真实通过，不替代台式机原失败根因收敛；parent CI仍有格式与macOS Intel失败、MSRV待终态，不能声明整个Windows或全平台生产就绪。

CI最终状态补充：`37880305608`所有job已终态。Windows stable与MSRV1.97均success；整个run为failure，仍有rustfmt与macOS Intel历史比较预算失败，打包skipped。完整元数据见 `ci_2552d082_passed_targets_2026_10_09/run_terminal.json`；因此现在可正常推送下一批修复而不取消原验证。
