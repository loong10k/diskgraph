# 删除后原句柄资格核验 / Original-handle post-mark seal

源码 9975288a29f3f90169525e449891332bcc35d7b3，原生 Windows CI 37433296989 终态 success。逐案检查全部 82 项原始 stdout：真实精确用例执行，各为 1 passed / 0 failed / 0 ignored；核对冻结候选摘要和两项新验收标记。

最终校验之后、原生提交删除之前真实创建硬链接，提交后的原句柄报告剩余链接，seal 拒绝完成并保留当前子项/删除责任；原错误不改判为成功。真实空子目录成功提交后，原句柄完整身份、类型、delete-pending 与零链接状态通过核验，最后关闭后的原身份通知确认和 EOF 均通过。

原始 receipt、构建日志和全部逐案输出保存在 original_output.tar.gz，摘要见 verification.json。此前原代码真实行为 RED 及原生 metadata 诊断另存 windows_hardlink_race_red_2026_10_06 和 windows_post_mark_metadata_2026_10_06。不是当前全 workspace 验收。

生产递归 owner/Pool/实际任务接入、seal 成功后再次变更、通知丢失恢复、有限退出及其余平台门禁仍未完成；危险操作不启用，生产父任务不勾选。

All 82 exact native Windows cases actually passed on source 9975288 in run 37433296989. The real last-interval hardlink race is refused by the original-handle post-mark seal; the real empty-directory positive control also passed. Original logs and receipt are retained with hashes. This qualifies the isolated primitives only. Recursive product owner/Pool integration, mutation after the seal, notification-loss recovery, finite shutdown and full-platform production acceptance remain open.
