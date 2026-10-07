# 监督准入错误的稳定分类

沿用既有监督准入合同，未确认原槽须报告 `recovery_unconfirmed`，不得混入普通锁冲突、成功或任意 Unsupported。新增 BusinessError::RecoveryUnconfirmed，CLI退出8，与 needs_attention 同属需要人工核对的失败，但稳定wire代码独立；既有代码/退出映射保持不变。

真实 SlotError 分类：Busy→resource_exhausted/7；Unconfirmed→recovery_unconfirmed/8；InvalidRecord→needs_attention/8；Unsupported→unsupported/6；Deadline→budget_exceeded/7；Io保留原I/O对象并沿既有入口分类。清理包装不得改变主错误码与退出码。此接口供实际监督入口使用；单独分类实现不证明准入已接入或数据库出生门禁通过。

兼容性：既有wire字段、API版本及错误码不变。公开Rust BusinessError新增变体会影响下游穷尽match，消费者须添加RecoveryUnconfirmed分支；不是完全源码兼容。未来发布必须遵循破坏性版本策略并携带迁移说明，本次不发布、不生成版本标签。

验证：新增接口测试先因缺少变体和From转换编译失败，补齐后Engine错误分类2/0、CLI原清理包装3/0、core错误表1/0。当前工作区workspace all-targets check、core/engine/CLI all-targets Clippy -D warnings通过。两路独立源码审查为APPROVE/WATCH，WATCH为上述外部Rust源码兼容影响；当前监督入口仍RED，未关闭平台或生产总门禁。
