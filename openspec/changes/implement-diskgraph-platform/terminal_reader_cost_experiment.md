# 终检连接配置成本实验：未采纳

沿用Q-08与终检授权合同，不改变50ms期限、过滤视图或独立新鲜连接。为诊断Windows超预算，实验用独立专用Reader省略通用cache/temp_store PRAGMA；真实WAL隔离、期限不出生、百万SQL中断3/0，macOS展示授权5/0、Linux实际扫描材料关系回归8/0。RED是新API不存在导致编译失败，不是已有性能回归复现。macOS关系测试因缺部署材料Unsupported而未通过，不替代Linux证据。

隔离Linux arm64 Rust1.97 release，实际固定图库连接+匹配+关闭，预热10轮，每种连接200样本，交替先后。初次通用/窄连接p50为182000/181458ns，p95为197250/193625ns。另三轮p50为182917/181833、183667/181417、183000/181708ns；p95为217750/220542、277625/260167、251917/283375ns。中位差仅约0.3–1.2%，尾部双向波动，没有证明可靠收益，更不能解释Windows50ms失败。

结论：撤回专用公共类型和产品接线，保留现有终检连接与全部安全语义，不增加无已证实收益的API。实验patch和完整日志保存在docs/benchmarks/supervisor_startup_admission_a08/terminal_narrow_reader_experiment.diff、linux_terminal_narrow_reader.log与linux_terminal_narrow_reader_repeated.log。实验源码基于08aad89归档及候选7文件，额外overlay实验patch；Linux关系回归使用已固定7fc实际扫描镜像，不构成当前扫描镜像验收。bench无需扫描，测量只支持连接开销结论，不能作为20k/200k全平台性能验收。

Windows预算失败仍待原生诊断；生产父任务不勾选。

## 内存报告独立修复（待原生验收）

当前 release 物理夹具在非 Unix 平台或 getrusage 失败时输出 RSS=0，无法区分未观测与真实零。保持已有字段名称，测量失败或缺少原生实现时输出 null；Windows 主进程读取 PeakWorkingSetSize（字节），退出子进程仍未知。父/子独立高水位和仅在两项都已知且加法不溢出时输出；它不是同时 RSS，也不是严格上限。Unix ru_maxrss 是进程生命周期高水位，不称为扫描阶段新增内存。验收包含 OS 调用失败、未知合计、溢出、原生当前进程正值；Windows 原生未运行前不得标记完成。

本批本机验证：旧函数注入 getrusage 失败 0/2，新函数同故障 2/2；macOS 实际模块 2/2，报告消费者与冻结夹具 overlay 回归 33/33。消费者对未知或非有效整数输出 null，保留合法负差；精确夹具清单加入新模块，历史产品源码摘要未修改。独立代码复审 APPROVE、架构复审 CLEAR。整体基准和 Windows 原生仍待 CI。

Windows API 依据：[PROCESS_MEMORY_COUNTERS](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters)、[GetProcessMemoryInfo](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo)。

### Windows 200k 外层超时诊断与原异常保护（2026-10-08）

原生打包任务 113089493625 / CI 37708407574 仍失败，不能判定生产就绪。负载 stderr 新增固定阶段与单调时间，含创建、注册、index、覆盖核验、查询、预算拒绝；外层 TimeoutExpired 重放捕获日志并传播同一异常。300 秒期限不变。包装工作区成功时回收，异常时保留并报告路径，以免未知后代仍占用文件时删除失败掩盖原异常。保留材料不代表进程退休完成，也不解决性能根因。

回归：日志丢失 RED 10 项中 1 失败 → GREEN 10/10；工作区保留 RED 11 项中 1 失败 → GREEN 11/11；负载覆盖 5/5。仍待新提交 Windows 原生阶段日志与进程生命周期验收，不勾选父项。

Windows 验收 Job 候选继续开放：新增 scripts/windows_acceptance_job.py 与契约/原生候选测试。当前仅本机生命周期替身 3/3、Windows 原生 1 项未运行。尚未接入打包入口：需先解决 Python subprocess.run 超时后 communicate 等待后代继承管道的问题；必须在等待管道前终止原 Job，使用有界退休检查并保留原异常。不能把候选当作已实现或原生通过。

Windows 验收 Job 候选已补齐有界管道退出并接入打包入口：Windows 分支持原无名 Job，以显式 handle_list 传继承副本；可信包装器先绑定自身再执行目标脚本。执行沿原300秒绝对期限，清理使用独立5秒期限。Job未知仍尝试原Popen终止；所有句柄独立释放，清理错误附加原异常或使正常路径失败。模拟契约7/7、包装11/11、负载5/5；两路复审APPROVE/CLEAR。新增Windows原生后代启动/timeout回收测试在打包前运行，尚未执行；产品嵌套Job、200k负载与全部平台门禁保持开放。SDK依据：https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects 与 SDK Job基础计数/扩展限制结构。
