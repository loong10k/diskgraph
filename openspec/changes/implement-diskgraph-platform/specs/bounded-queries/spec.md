## Purpose

使 CLI、MCP 与嵌入式消费者能够以有界、结构化且带时效说明的结果查询目录关系，支持历史比较与审阅候选，并在权限不足、版本过期、结果截断或证据缺失时提供明确反馈。

## ADDED Requirements

### Requirement: Q-01 Shared query semantics
系统 SHALL 提供 status、explore、search、node、children、top、related、explain、impact、snapshots、changes、growth、candidates 的一致查询语义，不为 MCP 与 FFI 维护不同事实版本。

#### Scenario: Equivalent entry points
- **WHEN** 同一身份通过不同入口提交相同 scope/revision/过滤参数
- **THEN** 返回语义一致的对象、尺寸、覆盖与证据，允许展示格式不同。

### Requirement: Q-02 Bounded traversal and output
查询 SHALL 限制深度、节点/边数、时间和返回字节，采用稳定分页；截断时返回原因及继续方式，不能把前 N 项称为完整结果。

#### Scenario: Large directory
- **WHEN** 目录超过请求预算
- **THEN** 返回有界结果与游标，不加载或输出整棵树。

### Requirement: Q-03 Explainable explore
explore SHALL 使用明确定位、模式和过滤条件聚合主要子项、尺寸、关系摘要、证据与下一步 ID；自然语言解释由宿主完成，名称歧义不得静默猜选。

#### Scenario: Ambiguous project
- **WHEN** 搜索名称对应两个不同项目
- **THEN** 返回候选与限定信息，不选任意项目并继续形成删除计划。

### Requirement: Q-04 Historical comparison
changes/growth SHALL 检查 server/scope、卷/provider、扫描设置、口径及覆盖的可比性；不兼容返回原因，未覆盖不作确定删除，首版不默认推断重命名。

#### Scenario: Different volumes
- **WHEN** 相同路径的两次快照来自不同卷
- **THEN** 拒绝直接计算可信增长并说明卷不一致。

### Requirement: Q-05 Conservative candidates and impact
candidates SHALL 区分 eligible_for_review、blocked、unknown，要求明确重建依据与保护/占用及后代检查；impact 按关系专属传播规则计算已知影响，两者均不授予操作权限。

#### Scenario: Protected descendant
- **WHEN** 候选祖先包含受保护或占用后代
- **THEN** 阻断该祖先，返回解释，不因父目录名为 cache 放行。

#### Scenario: Cannot reach requested bytes
- **WHEN** 安全审阅候选不足目标大小
- **THEN** 报告缺口，不扩大范围、忽略未知或将共享空间重复累加。

### Requirement: Q-06 Freshness and recoverable outcomes
查询 SHALL 返回 revision、观察时间、覆盖、未知原因和新鲜度；未建索引、无匹配、需同步、拒绝访问与服务故障具有可区分结果，普通查询不得隐式启动全盘扫描。

#### Scenario: No index
- **WHEN** 首次 explore 访问尚未索引的授权 scope
- **THEN** 返回 not_indexed 和明确下一步，不自动索引。

#### Scenario: Stale evidence
- **WHEN** 索引存在但相关观察过期
- **THEN** 显示 stale/needs_sync，不将缓存结果当作实时状态。

### Requirement: Q-07 Safe structured envelope
v2 SHALL 为不透明 ID、字节数和跨语言不安全整数提供无损编码；显示文本与原始定位分离，未知值不能用零代替，文件名和证据文本始终作为不可信数据。

#### Scenario: Large byte count
- **WHEN** 大小超过 JavaScript 安全整数范围
- **THEN** CLI/MCP/Swift/Kotlin 往返不损失精度且 v1 数字类型不被无声改变。
