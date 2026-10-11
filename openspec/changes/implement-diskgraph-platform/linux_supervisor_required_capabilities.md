# 生成监督服务的最小出生与回收能力

当前 bded132c CI 38175857890 的 Linux MSRV 真正 systemd 服务报告 phase 17 / setresuid / EPERM。原 MainPID UID/GID 全 0、CapEff=000001ffffffff7f（缺 bit 7 / CAP_SETUID）、NoNewPrivs=1、Seccomp=2、无 LSM 限制标签。原日志保留，不把旧 seccomp 错误文本当失败原因。

systemd v255 exec-invoke.c 与主线都在 keep_seccomp_privileges 路径且 ambient 未请求 CAP_SETUID 时显式 drop CAP_SETUID；这与实测吻合，是配置修复依据，尚未用上游版本替代 runner 实装版本证明。
https://github.com/systemd/systemd/blob/v255/src/core/exec-invoke.c

验收：生成 unit 显式 AmbientCapabilities=CAP_SETUID，能力上界只保留 CAP_SETUID、CAP_SETGID、CAP_KILL。前两项用于原 C 子进程清辅助组与降 UID/GID，后一项用于原监督 owner 向降权后子进程发回收信号。禁止添加 DAC 绕过、ptrace、SYS_ADMIN、SETPCAP 等能力；NoNewPrivileges=yes、ProtectSystem=strict、AF_UNIX 限制及原恢复期限保持。原 C 角色在 exec 前清 ambient、清保留能力、完成降权、归零 capabilities；业务 Engine 不以 root 运行。

原真实服务验收必须检查 MainPID 实际 CapEff/CapBnd 仅上述三项（0xe0）、CapAmb 仅 SETUID（0x80），以及原 standalone frontend doctor 成功、原槽 clean、原 owner 退休。不能以 unit 文本或纯 Python 测试代替。

Python 回归先失败 18 通过/1 失败，缺少明确能力配置。实现与实际 service 验收完成前不勾选三平台父项。桌面服务不安装。

为避免把 CAP_KILL 仅凭静态说明视作已验证，C driver 增加 signal_after_ready 场景：等待实际降权后的原 Ready，使用原 pidfd 发 SIGKILL，并消费原 wait；不得以 PID 猜测或 ACK 后自然退出充当信号成功。无 CAP_KILL 的负向输入仍发送原 ACK，让原 child 自然退休且原 owner 不遗失。此为出生前原语，不代表 Engine 异常 I/O 回收通过。

## 限定环境已验证

本机 Python 19/0、broker wire/cleanup 5/0、CLI 父节点预算回归 1/0、CLI all-target Clippy 通过。同步旧 CLI 流程断言：预算 1 返回父节点且 0 孩子，预算 2 返回父节点加 1 孩子，字段及 node_limit 原因保持；原 CI 已复现旧断言失败 15/1，没有修改生产实现来迁就旧断言。

已有 Zig 0.16.0 编译当前 C 夹具，Linux ARM64 无网络/只读临时容器实际四场景满足预期：缺 SETUID 的 C 子进程 phase 17 / EPERM 且原 wait 消耗；三个能力齐备可降至 UID/GID 23111、组 0、capabilities 空且正常退休；缺 KILL 的原 pidfd 信号 EPERM，推进原 ACK 后自然退出并消耗原 wait；三个能力齐备原 pidfd 信号成功且原 wait 确认 CLD_KILLED / SIGKILL。无新工具安装，无桌面服务安装。

证据 docs/benchmarks/linux_supervisor_required_capabilities_2026_10_12/。原 bded CI 的生成服务失败原样保留；修复后的真正 systemd、Linux x86、三平台服务及 Engine 异常 I/O 验收仍未完成。

## 当前源码的真实 Linux x86 MSRV 服务验收

CI38179170488、源码2a6e2db60104e676a88b83e3538e25fa747304eb、原job114587860136的原生监督步骤成功。已下载并逐字段核对原artifact11716879880：实际systemd MainPID CapEff/CapBnd/CapPrm=0xe0，CapAmb/CapInh=0x80，NoNewPrivs=1，Seccomp=2。实际生成unit启动，原frontend doctor退出0，原槽FD在stop之前CLEAN，未boot-enable、未安装桌面服务。该环境原phase17缺SETUID失败已修复，公开broker18场景通过、0失败。

原完整回执、能力观察和原日志保留在 docs/benchmarks/linux_systemd_minimal_capabilities_native_2026_10_12/。只关闭此Linux x86 MSRV生成服务的能力与普通退休子验收；整轮CI尚未结束，运行中异常Pending owner未注入，macOS/Windows服务以及后续e21e80d1终检窗口候选均不能由此证明。三平台和异常I/O父项仍不勾选。

Linux x86 stable原job114587860081/artifact11717179299和ARM64 stable原job114587860276/artifact11717470534已同样逐字段验收：两者均18/0、实际能力0xe0/ambient0x80、生成unit doctor成功、原槽在stop前CLEAN。与MSRV原回执一起保留于上述目录/native_matrix.json。此更新仍不证明异常Pending owner、macOS/Windows服务或后续scope终检候选。
