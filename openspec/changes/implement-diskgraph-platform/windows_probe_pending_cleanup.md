# Windows 探针暂时清理未完成

沿用 PF-06 的原 owner、容量、取消与绝对期限合同。当前成功收集完整输出后只执行一次非阻塞清理观察，暂时 Pending 立即变成失败。修复应仅在输出成功且原预算仍有效时，用同一个 owner 和绝对期限继续观察；每次等待最多 1ms，不延长期限，不重新认领容量。真实清理错误、取消、超时及 panic 保持原 owner 移交及原主错误。

验收先在真实 Windows 生产探针路径暂缓一次 wait 观察，确认旧实现返回 Pending 错误；修复后须实际观察 leader 退出、全部 Job 进程为零、管道完成及容量释放。该保守暂缓注入不是内核故障证明，不得伪造退出、EOF 或 Job0。原取消 wait/query 失败、panic、到期不进入旧阻塞清理等原生回归必须保留。全量并行、Git 15秒及全平台门禁仍需独立验证，本项定向通过不能代替生产就绪。

## 2026-10-09 Windows 原生证据

台式机 Rust 1.99.0/MSVC：新回归在旧产品实现实际失败，错误为 `probe cleanup pending; original owner retained`；修复后 1/1 通过。整理代码结构后完整恢复组 5/5 通过（含原取消 wait/query 故障、panic 和到期不进入旧阻塞清理），Clippy lib/tests `-Dwarnings` 通过。最终产品文件 SHA-256 为 `435cc69e2b5a69b0974bb22474d582aa41cf19ad964d452ffec1b5c22d2f60e2`。归档 `docs/benchmarks/windows_probe_pending_cleanup_2026_10_09` 保留原日志、脚本和逐次源码摘要。RED 外层包装误用 CompletedProcess 作为退出值，外层记录1；内层 Cargo 实际101及目标断言失败已保留。GREEN及后续包装已正确传播 returncode。

默认并发 Engine 全量正在运行；其900秒测试宿主窗口仅供完整收集结果，不改变生产 Git 15秒预算、授权50ms或200k全流程300秒验收。不得将未结束的全量算作通过。

旧提交 `61283557` 的 CI `37820467541` 已终态：19成功、4失败；Windows stable/MSRV Engine 各703通过、1失败、4忽略，均为 relation 数据期限耗尽后终检授权预算；另有 macOS Intel history 与 Windows 200k 包失败。原始日志及终态见 `docs/benchmarks/ci_61283557_terminal_2026_10_09`。它们不是本修复后的同提交验证，仍保留为开放门禁，不因本次定向通过被覆盖。
