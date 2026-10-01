## Purpose

在索引查询之外提供可选择启用的文件执行能力，要求计划、可信批准、实时重验、逐项记录和可验证恢复条件，保护用户文件并区分回收、永久删除及实际磁盘空间变化。

## ADDED Requirements

### Requirement: OP-01 Read only by default
文件修改 SHALL 默认关闭；启用受控执行不改变查询的只读语义，不开放任意 SQL、Shell、自动提权或无范围路径删除接口。

#### Scenario: Default deployment
- **WHEN** 新安装服务收到 move/apply 请求
- **THEN** 拒绝修改并说明未启用执行能力。

### Requirement: OP-02 Immutable explicit plans
操作计划 SHALL 绑定主体、server/scope、revision、精确源/目标、对象指纹及目录成员边界、动作、数量/字节预算、策略版本、期限和恢复条件；预览不修改目标资源。

#### Scenario: Nested targets
- **WHEN** 请求同时包含父目录和其子目录
- **THEN** 计划去除重叠或拒绝歧义，不重复执行或重复统计空间。

### Requirement: OP-03 Trusted approval
apply SHALL 验证可信人机交互或管理员预授权策略产生的批准，批准绑定计划摘要、主体、动作和有效期；模型传入 approved=true 或自由文本确认不得有效。

#### Scenario: Forged approval
- **WHEN** 智能体自行构造批准字段或替换计划目标
- **THEN** 拒绝执行且记录拒绝。

#### Scenario: Expired approval
- **WHEN** 批准过期或策略已撤销
- **THEN** 要求重新审阅，不按旧许可继续。

#### Scenario: Scope revoked after approval
- **WHEN** scope 或动作授权在批准后、apply 前或批量执行途中撤销
- **THEN** 未执行项不得修改文件，操作记录准确区分已完成项和被阻断项。

### Requirement: OP-04 Execution revalidation
执行 SHALL 重验源/目标身份、范围、保护、占用覆盖、目录内容、挂载和覆盖冲突；变化返回 stale_plan，安全前提未知则阻断，不仅比较展示路径。

#### Scenario: Race on destination
- **WHEN** 计划后目标路径被创建或替换为符号链接
- **THEN** 不覆盖或越界，拒绝或安全中止。

#### Scenario: Protected child added
- **WHEN** 批准后目录新增受保护后代
- **THEN** 停止该目录操作并要求重新计划。

#### Scenario: Same inode changed after approval
- **WHEN** 已批准源文件保持 inode 但大小、修改时间或内容变化，或超过计划字节预算
- **THEN** 拒绝执行并要求重新计划，不以执行时的新大小重写旧批准。

### Requirement: OP-05 Move and copy contracts
move/copy SHALL 默认拒绝覆盖并校验源目标授权；同卷移动与跨卷复制后删除分别处理，跨卷流程在完整校验目标前不得删除源，失败不得伪装原子完成。

#### Scenario: Copy interrupted
- **WHEN** 跨卷移动复制到一半崩溃
- **THEN** 保留源和可追踪 staging，重启后报告未完成。

#### Scenario: Unsupported metadata preservation
- **WHEN** 平台不能满足计划要求的权限或扩展属性保真
- **THEN** 明确阻断或要求重新批准降级计划，不静默丢失。

#### Scenario: Unauthorized copy destination
- **WHEN** 源 scope 有授权而 Copy 目标未注册或未授予对应动作权限
- **THEN** 计划及执行均拒绝，目标不产生文件。

### Requirement: OP-06 Trash and recovery records
trash SHALL 在可验证的回收能力下保存恢复条目、原位置、资源身份与元数据，禁止回收失败自动退化为永久删除；restore 必须重新计划并校验原位置冲突和权限。

#### Scenario: No safe trash backend
- **WHEN** 平台或卷不支持配置的回收方式
- **THEN** 返回 unsupported，不执行永久删除。

#### Scenario: Restore collision
- **WHEN** 原位置已有其他文件
- **THEN** 不覆盖该文件，提供新位置重新计划或取消。

### Requirement: OP-07 Irreversible purge
purge SHALL 具有独立权限与明确不可恢复说明，仅处理已批准精确对象；永久删除后不得声称可通过图快照恢复文件内容。

#### Scenario: Restore after purge
- **WHEN** 用户请求恢复已经永久删除且无备份的对象
- **THEN** 明确不可恢复，不生成虚假成功记录。

### Requirement: OP-08 Durable idempotent execution
执行 SHALL 在文件副作用前持久记录意图，并将幂等键绑定主体与计划；相同重试返回同一操作，冲突键拒绝，崩溃恢复不得盲目重放不可逆步骤。

#### Scenario: Lost response retry
- **WHEN** 客户端对同一计划与幂等键重试 apply
- **THEN** 只存在一个操作；返回已有状态，不再次移动或删除。

### Requirement: OP-09 Partial completion and cancellation
批量执行 SHALL 保存逐项状态，冲突资源串行化或拒绝并发；取消停止后续步骤并记录已完成项，不承诺数据库和文件系统跨资源原子事务。

#### Scenario: Cancel midway
- **WHEN** 部分文件已回收时收到取消
- **THEN** 报告部分完成及可恢复项，不将已执行项标记未执行。

#### Scenario: Concurrent overlapping operations
- **WHEN** 两个已批准操作并发声明相同源、重叠目录或同一目标
- **THEN** 仅一个操作能在持久事务中认领资源，另一个返回冲突且不发生文件副作用。

### Requirement: OP-10 Verify space and index
执行后 SHALL 刷新受影响观察并分别报告逻辑处理字节、回收区保留字节和卷空闲前后值；并发活动、快照/硬链接/打开句柄影响必须作为测量限制说明。

#### Scenario: Trash on same volume
- **WHEN** 大目录移到同卷回收区
- **THEN** 不得声称按目录大小释放了磁盘空间。

#### Scenario: External writes during cleanup
- **WHEN** 执行期间有其他进程写入磁盘
- **THEN** 实测差值附限制，不把估算大小当作净释放量。

### Requirement: OP-10 Bound staging and atomic publication
The system SHALL bind staging creation, file writes, publication and cleanup to directory handles, exclusively create temporary files, verify content digests and required metadata before publication, and use native atomic no-replace publication. Unsupported fidelity conditions SHALL be refused. CLI and MCP dangerous tools SHALL remain disabled regardless of library tests.

#### Scenario: Two publishers race for one target
- **WHEN** two verified copies attempt publication to the same target
- **THEN** exactly one succeeds and the other reports a conflict without overwriting the winner

#### Scenario: Copy parent is replaced
- **WHEN** the target parent path is replaced with a symlink after verification
- **THEN** publication and cleanup operate on the fixed original directory handles and do not touch the replacement target
