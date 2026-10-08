# 授权预算失败的阶段诊断

当前 Windows stable 历史比较和 macOS Intel 三项正向查询预算失败仍未关闭；已有回调计时不能确定终检哪个 SQL/准备阶段失败。

本次为关系/历史末段的控制锁准入、控制授权 SQL，以及新鲜归属终检的 server SQL、reader 准备、ownership SQL 接入失败诊断。仅 debug 构建且 `DISKGRAPH_QUERY_DIAGNOSTICS=1` 启用；成功、普通拒权不输出，预算/SQLite busy/interrupted 失败只输出固定阶段标签与微秒数，不输出错误文本、资源、主体或 token。清理包装、主错误及原截止时间保持；不重试、不缓存归属、不刷新 50ms。stderr 写入失败不替换原错误。

此诊断不覆盖所有能力回调、初始化或其他查询分支，不能因没有阶段日志推断它们成功。CI 原 full workspace/all-targets/no-fail-fast 命令保持，仅显式开启 debug 失败诊断。Release 不启用，不以诊断实现代替故障修复。

本机验证：诊断测试 3/3，覆盖关闭时不输出、成功/拒权不输出、预算错误只输出固定标签并保留原清理 payload；原终检控制锁立即拒绝回归 1/1；跨 scope/root 正向历史比较 1/1。关系模块运行 17 项，其中 9 通过、8 在受保护扫描安装夹具初始化返回 Unsupported，未进入目标行为，不能计为通过，也不把此环境故障作为新行为红灯。Engine 全 targets Clippy、fmt、OpenSpec strict 通过。默认 python 缺少 PyYAML，改用已安装的 Anaconda Python 完成 workflow YAML 解析，无安装变更。

原生后续验收必须对应推送后的同一 SHA；保留旧原生失败和原业务成功断言，任何 task 不因此勾选。

最终接线后再执行 `history_scope_eligibility` 全目标 11/11 和全平台源码 AST 门禁 6/6，通过；诊断显式启用，没有修改测试断言或延长观察期限。旧 CI `37765612472` 已终态：20 个 job 成功、3 个失败。Windows MSRV job `113273033534` 成功（Engine unit 674 通过、0 失败、4 忽略）；stable 的失败保持。终态矩阵、MSRV 原日志及摘要保存在 `docs/benchmarks/terminal_ci_2e7b35a_2026_10_08/`，仅对应旧 SHA `2e7b35a`。
