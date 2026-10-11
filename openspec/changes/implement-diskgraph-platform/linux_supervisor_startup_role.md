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

## C 层受控降权失败夹具

原 root driver 增加 denied_groups 场景：driver 在本次独立进程中安装仅拒绝 setgroups 的过滤器，原 child 经过固定 Init 后真实调用 setgroups 并返回 phase 15 / EPERM。验收必须同时确认消息 kind 2、12 字节、公开描述符已关闭、未 ACK/exec、原 pidfd wait 消耗与正常 exit 127。此过滤器只用于隔离夹具，不改变生成 systemd 服务策略。现有 clean、身份拒绝、坏镜像及 baseline 场景保持。Python 语法和差异检查通过；C 编译及该场景执行等待 Linux CI，不记为本机已通过。

## 隔离 Linux ARM64 C 原语实际结果

使用本机已有 Zig 0.16.0、静态 aarch64-linux-musl、Wall/Wextra/Werror 编译当前 driver、role 与原 C 源码。使用已有不可变镜像 sha256:29e8589c3f9ba699b5f7aa4b3c7733c58852a3626439e619aa0ee78de08c6ca0，临时容器无网络、只读系统/夹具、仅 SETUID/SETGID 能力，无新安装，无桌面服务。

实际 clean：UID/GID 23111、辅助组 0、capabilities 空、私有端点与继承白名单正确、no_new_privs，原 pidfd wait 与 exit 0。实际 denied_groups：kind 2 / 12 字节 / phase 15 / errno 1，未 ACK，原 pidfd wait 与正常 exit 127。旧普通 atomic baseline 实际复现 root UID、非空能力、无私有端点并返回失败；三项输入检查均满足预期。证据 linux_arm64_container_c_receipt.json 含原 stdout/stderr、返回码、镜像/产物/源码摘要。

这只补强 Linux ARM64 C 出生原语，不证明 Rust owner、Linux x86、真实 systemd、macOS/Windows 服务或异常 I/O 回收通过。前段“C 编译/执行等待”已由本段在该限定环境补充，原生 CI 服务验收仍待完成。
