# Linux 监督服务启动错误角色与阶段

延续原三平台监督服务验收，不缩小父项或启用桌面服务。

## 可观察验收行为

- 普通原子子进程 phase 8 仍表示 seccomp，接受阶段仍为 1–11。
- 监督服务旧 phase 8 表示降权，不能再报告为 seccomp；新 phase 12–18 分别标明 keepcaps、ambient、no_new_privs、setgroups、setresgid、setresuid、capset。
- 固定 12 字节消息、原 errno、原请求期限、检查点和 pidfd 回收契约不变。角色由受信启动器选择，不从客户端字段接受。
- 启动发送失败与原 socket ECONNRESET 恢复均采用同一角色校验；非法消息不得覆盖传输错误或成为 Ready。

## 已验证与待验证

本机先确认 1 通过/1 行为失败：监督服务降权误标为 seccomp。修复后纯消息回归 4/0；Engine 全平台源码结构 6/0、macOS all-target Clippy、fmt 和 OpenSpec 严格校验通过。

新增 2 项 Linux 原 socket reset 回归，等待原生 Linux CI；本机纯消息测试不证明 Linux C 启动或真实 systemd 安装通过。

37253b8d 的 CI 38172485982 原 systemd 失败仅证明旧 phase 8 的降权失败，尚未确认具体 syscall。原始日志保持原样；之前日志中的 seccomp 文本是解码器误标，不能作为实际 seccomp 安装失败的证据。下一轮沿用原生成服务，输出具体阶段及 MainPID 能力/安全策略观察，不移除保护或放宽期限。

证据：docs/benchmarks/linux_supervisor_startup_role_2026_10_12/receipt.json。三平台服务、异常不可回收 I/O、完整 CLI/MCP 父项保持未完成。
