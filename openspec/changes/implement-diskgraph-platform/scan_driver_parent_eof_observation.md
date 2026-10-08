# 父驱动 EOF 边界的真实测试观测

Windows 默认并发 Engine 回归在 `caller_cancel_after_tree_eof_still_rejects_tree_and_cleans_actual_child` 中复现：子端已写出 `pipes-closed`，但父端下一次取消 poll 返回 `Ok(None)`。真实管道关闭与父端异步读取完成不是同一事实；原测试仅依赖子端文件，未证明其标题要求的父端 EOF/树组装边界。

验收保持原 20 秒绝对期限、真实子进程、默认测试并发和原取消/回收断言。测试必须等待父端已经真实消费 EOF、完成完整树组装，且此前每次 poll 仍为 Pending；随后请求取消必须拒绝交付树、保留 Cancelled 原因，并确认原进程与 Job 实际回收。仅在测试构建暴露只读组装状态，不改变产品驱动、原生读取、清理或退出许可。

本项只修正边界观测，不把完整 Engine 的其他 98 项失败解释为误报，不关闭 Git 15 秒、Windows 200k/300 秒或同 SHA 全平台门禁。未勾选整体完成。

Windows Rust 1.99 原生任务 `wc_job_sulMsYTZozT0a9sn`：从当前源码重新构建独立真实夹具后，父驱动测试默认并发 12/12 通过（含原七个冻结测试），测试 0.65 秒；Engine 包格式检查通过。源码、夹具产物及日志摘要见 `docs/benchmarks/windows_driver_parent_eof_16d1_2026_10_09.json`。这是定向回归，完整默认并发尚需复验，不能将它报告成 99 项全量失败已消除。
