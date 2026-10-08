# 管道缓冲源码规范验收（2026-10-08）

沿用 RT-09 源码组织与中文公共契约要求，不更改正式验收条件。

CI 37731786365 在提交 1fb077c 上的 Linux x86 stable/MSRV、Linux ARM stable、macOS ARM stable/MSRV 共五个终态失败任务均由同一规范缺漏触发：新增 `WorkerOutput::for_pipe` 文档使用“参数为”，缺少源码门禁要求的“参数：”。五份完整任务日志中未发现其他 `... FAILED` 测试记录。

修复补充明确的 output、limits 参数说明及已有返回说明，不改变运行行为，不放宽门禁。修复后本地 `source_layout` 六项全部通过，CI 对应 workspace 包格式检查通过。完整工作区 `cargo fmt --all` 会检查 vendored disktree 的上游格式差异，本轮保留其源码与摘要，按项目 CI 的明确包列表检查。

证据：`docs/benchmarks/worker_pipe_source_contract_2026_10_08/verification.json` 及五份压缩原始日志。仍需新提交的实际平台 CI；该局部修复不证明生产就绪。

当前 CI 尚有 Windows 完整负载诊断及其他原生任务在运行；为保留这些实际观测，修复提交暂不推送触发自动取消，待其终态后推送。
