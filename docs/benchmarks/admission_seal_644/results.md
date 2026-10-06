# 原资源池准入关闭与 CI 源码规范修复

沿用 implement-diskgraph-platform/frontend_recovery_supervisor.md。关闭与新预留使用同一原状态锁；关闭永久且幂等，已有预留/session 不被丢弃。占用数归零不能代替原 owner 回收，尚未接入 CLI/MCP 监督生命周期。

本机 macOS arm64：新扫描容量测试真实 RED 0/2 → GREEN 2/0；扫描资源回归 7/0；监督槽回归 7/0（一个 child 入口仅由真实父测试显式调用，单独标记 ignored）；源码组织 RED 5/1 → GREEN 6/0；Engine all-targets Clippy 通过。Python 适配 RED 5/2 → 全 suite GREEN 245/0（已有 mock 文件句柄 ResourceWarning 未抑制）。Windows probe session 关闭测试仅 cfg(windows)，不声称本机验收。

旧 Windows pool 缺失新接口时仅补明确 Unsupported 拒绝；单独使用标记在目标原生 RED/GREEN 均被拒绝。旧清理正文与原恢复委托不变，不能借用新成功关闭行为。

已保存 644562a 的 Linux stable 实际终态失败：run 37546134410/job 112550485571，原门禁发现两个 pub(super) 方法缺少中文参数/返回契约；本机原门禁复现同因并补齐注释，没有削弱测试。最终源全平台 CI 尚未完成。

receipt.json 绑定源码与原始日志 SHA256；压缩仅用于存档，不修改原日志正文。此子组件不能证明有限前端退出、受保护容量命名空间或 ACTIVE 安全退休。
