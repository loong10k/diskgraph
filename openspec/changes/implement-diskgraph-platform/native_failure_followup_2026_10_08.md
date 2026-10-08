# 当前原生失败与配对性能复核

本记录对应 CI `37765612472` 的源提交 `2e7b35a9f619340c184593c6226f557999132c38`，不代表随后本地提交的验收结果，不关闭任何生产门禁。

## Windows stable 新增失败

完整原日志及 SHA-256 位于 `docs/benchmarks/windows_stable_2e7b35a_2026_10_08/`。

- Engine 单测 673 通过、1 失败、4 忽略。`busy_output_does_not_pay_an_idle_interval_per_fixed_block` 实际完整退出耗时 1.6010678 秒，超过原 1 秒断言。尚未区分进程调度、管道完成观察、待机退避或清理成本，保留原断言。
- `history_compatibility_matrix` 9 通过、1 失败。`node_quality_and_type_replacement_matrix_keeps_unknown_bytes_null` 在 62.8532ms 返回 `Business(BudgetExceeded)`；授权回调六次完成于约 4.49ms 内，不能据此确定之后哪个终检阶段耗尽窗口。现有独立 50ms 授权窗口保持不变。
- macOS Intel 的三项正向查询预算失败及 Windows 包全流程 300 秒超时仍未关闭。Windows MSRV 当时仍运行，不将观察超时当作任务终止。

## 本机历史比较复现边界

`docs/benchmarks/history_local_contention_2026_10_08/receipt.json` 绑定本地源码 `d32a52b`、实际测试二进制摘要和 macOS ARM64。正向跨范围历史比较单次测试通过后，32 次独立进程、最多 4 并发运行均通过；各进程使用自身隔离数据库。该结果不是 Windows/Intel 复现，不关闭其失败，也不证明生产长时间运行稳定。

## Linux 配对性能

`docs/benchmarks/paired_linux_2e7b35a_2026_10_08/` 逐件核对 69 个原始产物的字节数及 SHA-256，并保留原收据和测量日志。两份 stdout 实为完整源码 tar 归档，超过适合 Git 的大小，不重复提交；其摘要及上游 artifact 定位保留在 summary.json。固定基线为 `2a2f8281f9f211b6632bdb26afdcd4a4fb21a44d`。

| 夹具 | 候选扫描耗时 / 基线，两轮 | 宿主扫描峰值 RSS 差异 |
|---|---|---|
| 20k 宽目录 | 0.93898 / 0.93353 | 两轮相同 |
| 200k 宽目录 | 0.90360 / 0.89963 | +12,369,920 / +11,956,224 字节 |
| 300 层深目录 | 1.58757 / 1.59333 | 两轮相同 |

200k 扫描快约 9.6%–10.0%，但宿主 RSS 增加约 11–12 MiB，深目录慢约 59%。子进程与宿主高水位独立，不相加声称并发峰值。窄读延迟与持续运行仍需独立验收；不能以其中一个指标改善宣称总体性能完成。

本记录不修改规格验收条件、不勾选 tasks、不启用危险文件操作、不改变 vendored disktree。
