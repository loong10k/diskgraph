## Purpose

定义 DiskGraph 从已有库基础到本地和远程产品的验收证据，覆盖协议、权限、数据安全、真实平台、故障恢复和智能体效率，并明确私有交付与未来公开发布的授权边界。

## ADDED Requirements

### Requirement: RE-01 Behavior driven completion
每个实现增量 SHALL 从对应规格场景建立失败测试、最小实现及回归证据；文件存在、编译、返回 HTTP 200、任务勾选均不能单独证明完成。

#### Scenario: Missing behavior
- **WHEN** 命令存在但输出 stub 或不满足负向场景
- **THEN** 任务保持未完成，不能计入已交付命令数。

### Requirement: RE-02 Security and fault gates
写能力发布 SHALL 通过越权、伪造批准、重放、路径竞态、崩溃、断线、并发冲突、满盘、部分完成和恢复冲突测试；真实用户数据不得作为破坏性测试对象。

#### Scenario: Unverified write adapter
- **WHEN** 某平台写适配器仅编译通过
- **THEN** 该平台保持写能力关闭直至实际门禁通过。

### Requirement: RE-03 Efficiency evaluation
系统 SHALL 对大目录定位、项目归属、历史增长等任务进行同模型同数据的多次对照，分别记录冷索引、热查询、同步、正确率、工具调用、返回/驻留上下文、时间及存储成本，不套用 CodeGraph 数字。

#### Scenario: One query benchmark
- **WHEN** 仅热缓存单次查询更快
- **THEN** 不足以声称整体提效；补齐准确性和累计成本比较。

### Requirement: RE-04 Platform and protocol proof
发行 SHALL 区分单元/集成测试、真实客户端、目标 OS、移动真机与升级恢复证据；三种传输分别完成兼容性验证，产物附版本/校验和/依赖许可信息。

#### Scenario: SSE format but wrong protocol
- **WHEN** 现代 HTTP 返回事件流但未测试旧 HTTP+SSE 客户端
- **THEN** 不能宣称旧版兼容交付。

#### Scenario: Mobile build only
- **WHEN** 生成 Swift/Kotlin 绑定但未真机调用
- **THEN** 仅声明绑定生成完成，不声明移动功能可用。

### Requirement: RE-05 Private release and explicit stages
项目 SHALL 保持私有并按阶段启用已验收能力；公开仓库、公开包发布、生产部署或破坏性管理必须另获明确授权，规划完成不触发实施。

#### Scenario: Planning validated
- **WHEN** OpenSpec 文档与任务通过校验
- **THEN** 只说明规划完整，所有未执行实现任务仍未勾选，不自动发布或 apply。
