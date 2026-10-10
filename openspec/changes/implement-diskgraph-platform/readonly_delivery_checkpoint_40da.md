# 40da9ce4 只读交付验证状态

同SHA CI已终态：37969595796，23作业中21成功、两个Linux GNU包失败。Windows stable与1.97原始job日志已下载并按测试摘要提取，Engine各744/0/7ignored，MCP各233/0，无FAILED测试汇总；实际DLL拒绝码577。日志来源与摘要见ci_test_log_provenance.json及两份test_summaries.log。Linux x86/ARM、macOS ARM/Intel完整测试也成功；这不关闭Linux打包、监督退出链或长时验收。台式机独立失败仍保留，不以CI绿灯抹除。此前台式机另外观测到Cargo/Git后台进程，最新只读观察为零；已启动同binary、原预算、默认并发空闲复测，job wc_job_zAUp51Nf1W1R-BUg。当前连接工具不可用，不能把未观察终态的任务报告为通过、失败或停止。

沿用 implement-diskgraph-platform，不新增规格体系，也不关闭全平台生产门禁。

GitHub run 37969595796 的 Windows 原生只读包作业 113952899850 已成功。下载 artifact 11635396983，实际 ZIP SHA256 与附带摘要均为 ba50b755cf72b17e924e95b4c840d24a8331790aa717b4ad342a4c76b31735e4。包内 CLI/MCP 实际内容与 HTTP 验收前后指纹一致；worker 实际长度 865280、SHA256 8933924dea7aabc836d919fa74460f4c32ccecc729c7f57818585a1937d9e487 与包清单一致。上游 pin 仍为 158f9cc2f0b332194a3ffc5acec47760c99146d8。

真实 release 包测量：20k 文件扫描 8.012 秒，查询 p50/p95 22.661/30.471ms；200k 文件扫描 56.701 秒，查询 p50/p95 22.858/36.406ms。每例四客户端、32 次查询、6/6 检查通过，包含真实索引覆盖和超预算不发布。最终数据库文件长度分别 47370240/473374720 字节；不是累计写入、峰值 WAL 或 RSS。不能据不同运行器的单次样本宣称优化前后提升或回退。

HTTP 实际 14/14：匿名/恶意 Origin/无授权 token 拒绝、合法 scope 读取、实时撤权、legacy 与密钥文件准入。持续请求 60 秒、2322 次，p50/p95 20.693/28.037ms。报告 native_long_run_qualified=false，不把一分钟当成长时间 soak 验收。

另一个独立台式机 Debug 默认并发入口回归已结束：535.062 秒，退出101，没有1800秒超时，原 Job 退休确认。Engine库719/25/7ignored，MCP库233/0。Git失败多在目标后置边界之前返回Timeout；两项授权阶段也有前置失败。完整日志仍在固定远程隔离目录，不能据单项或MCP成功宣布默认并发整体通过。

证据目录：docs/benchmarks/windows_readonly_package_40da_2026_10_10/，保存原始HTTP/load报告、文件摘要来源及台式机实际终态回执。当前新增撤权测试诊断仅记录真实目标阶段是否到达，不更改预算、并发或PermissionDenied断言；须另行原生执行。

新增阶段诊断的原生结果：保持1000ms请求预算和固定授权窗口，历史撤权、关系撤权及终检重入三个精确测试均1/0（0.40/0.40/0.72秒）。随后同一binary默认并发Engine库682/62/7ignored，退出101；原Job退休确认、总302.265秒。新的历史诊断精确报告target withdrawal not reached: during_encode=true、BudgetExceeded，证明本次失败发生在实际撤权之前；不能将定向绿灯或该归因当成默认并发通过。隔离目录E:\\workspaces\\workspace-loong10k\\diskgraph_native_40da_phase_20261010，binary SHA256 ab22089dcfb0d74e6b603b21ddcd82f14328f727faa685c86d6a3cf3d9c2d580；仅增加测试阶段记录，生产代码未改变。

仍未完成：默认并发Engine失败根因与修复、Linux GNU glibc2.17打包、独立监督进程与有限公开EOF、同源码完整平台回归及缺失的长时间/RSS验收。Linux兼容编译器的安装候选尚未获得安装授权，未提交或执行该workflow修改。保持原非目标和写能力关闭。
