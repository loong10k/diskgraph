# Darwin 原 owner 退出过渡回归

对应 PF-06；不关闭完整平台生命周期、监督器集成或生产就绪父任务。

基线为 `027744a236a622388190569a1a60b8198aeaa108`。CI `37931001245` 的 Intel 回收用例曾返回原生 EPERM；同一用例在本机单独运行通过，不能据此认定问题已消失。新增固定 128 轮快速连续轮询，用真实私有 session child、真实 signal、真实组查询和原 wait 验收；修改前第一轮实际返回 EPERM。测试先确认原容量仍占用并清理原 owner，再报告失败，不遗留子进程。

单次 terminate 在 EPERM 后仍先沿用双重完整枚举的全 zombie 检查；若不能确认全 zombie，则另行核验双重完整组枚举、PID/PGID、原 leader 父身份及全部成员的 zombie/公开 INEXIT 状态。仅此退出过渡返回 Pending，保留原 owner、槽和绝对期限；它不会消费 wait、报告完成或发布。活动成员、缺失 leader、查询未知或枚举截断不符合例外，仍保留原错误。原注入权限失败回归也继续返回错误并保留原槽。

ABI 依据：[Apple proc_info.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h) 公开 `PROC_FLAG_INEXIT=4`，当前 libc 不导出该常量；[proc_info.c](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c) 将内核退出标记映射到该字段。它不是完成证据。组信号的 [kern_sig.c](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c) 排除 zombie 并在未找到可发送对象时可能返回 EPERM，不能将所有 EPERM 一概忽略。

本机 macOS ARM 验证：macOS 相关 12 项通过（含固定 128 轮实际回收、活动成员拒绝与已回收身份拒绝）；既有 Unix 生命周期 10 项通过；Engine all-targets Clippy、五个修改 Rust 文件的 fmt 和 OpenSpec strict 通过。新负向 fixture 首次遗漏 control close 导致正常退出许可拒绝，已修正 fixture；第一次编译因 libc 缺少常量失败，分别保留为 fixture/编译故障，不算行为红灯。

证据在 `docs/benchmarks/macos_recovery_exit_transition_2026_10_09/`，receipt 绑定实际修改文件 SHA-256，red_observation 明确为工具捕获记录，绿灯与静态检查有原始日志。验收未增加内部 sleep、未扩大原请求期限、未改 vendored scanner。Intel CI 尚未验证这批源码，Linux/Windows、GNU ABI、监督器集成和性能门禁均保持原状态；本机通过不证明全平台生产就绪。
