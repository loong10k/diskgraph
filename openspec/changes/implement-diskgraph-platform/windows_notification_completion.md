# Windows 原身份通知完成合同

## 问题与验收

923b6d8 原生 Windows Rust 1.97 job 112696120094 的完整测试在 Git 根清理确认失败，原 NTSTATUS 0xc000000d、Win32 87、ID 长度 8。附加 ID 查询不能产生通知型清理的成功证明，却提前中断原 REMOVE 通知观察。

通知型确认仅在删除前原订阅、原 held parent/child 身份、完整合法原 REMOVE 页及原 I/O 完成均成立时返回 true。尚无页或完整但不匹配的页返回 Pending；保持原 owner、订阅与容量。无效/丢失通知、身份变化和原 I/O 错误仍拒绝。独立 WindowsGitDeletionWitness 保留原正控与未知错误拒绝合同；87、权限错误或路径消失绝不是删除证明。

清理只在同一 ProbeBudget 的原绝对期限与取消约束内轮询；不在每次 Pending 重建预算。到期、取消或错误时保留原恢复 payload。测试必须覆盖实际外部原文件句柄阻止完成、Pending 不释放、关闭外部句柄后原匹配通知完成、陌生同名目录未被删除；独立 ID witness 的 present 正控保留。

## 验证状态

原完整 CI RED 已取得。更新后的通知型 Pending 回归及产品修复需要新提交 Windows 原生 CI，macOS check 不能证明此能力；验收尚未完成。原 SSOT 中禁止放宽生产 ID 查询的要求仍适用，本变更不修改 ID witness，而删除通知型确认中不能授予成功的附加 ID 查询。

## 恢复公平性

同步 cleanup 在一个原 ProbeBudget 内等待通知；有界 cleanup_until 每次推进到 Pending 即返回 Ok(false)，保留原帧及订阅，把执行机会交给后续槽。不得按错误字符串推断 Pending。同步等待使用 min(1ms, 原期限剩余量)。真实双槽公平性需要覆盖首槽外部句柄阻碍、次槽可完成、首槽仍占用以及解除后完成；该原生测试尚未执行。
