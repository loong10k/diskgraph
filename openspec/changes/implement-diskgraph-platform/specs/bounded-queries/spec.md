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

### Requirement: Q-08 Narrow reads and bounded historical comparison
The system SHALL decode only the requested page or bounded tree nodes, use independent SQLite read connections with deadlines, preserve Rust Unicode lowercase substring search, and bind v2 keyset cursors to principal, scope, revision, filters, actual ordering and policy version. Explicit offsets SHALL remain supported. Legacy cursors SHALL be rejected and require a fresh query. Ordered history merge SHALL retain no more than the output budget plus current iterator entries; truncated statistics SHALL be labelled partial.

#### Scenario: Small page avoids unrelated node decoding
- **WHEN** an unrelated node outside a node/top/search page contains an invalid locator
- **THEN** the requested page succeeds without decoding that node

#### Scenario: Wide tree and history exhaust the node budget
- **WHEN** the requested tree or historical comparison exceeds its node budget
- **THEN** the result reports its truncation reason and historical totals are not represented as complete comparison statistics

#### Scenario: High-degree relation and impact budget
- **WHEN** one entity has more relations than the remaining impact edge budget
- **THEN** the query reads only a bounded page and reports an explicit truncation reason rather than loading every edge or claiming a complete traversal.

#### Scenario: Explain and filtered related page
- **WHEN** related/explain asks for one edge from an entity with hundreds of relationships
- **THEN** only bounded pages and their corresponding evidence are decoded, the filter is applied before the limit, and the response identifies truncation and a stable continuation position.

#### Scenario: Wide TUI directory
- **WHEN** a directory contains more children than one TUI page
- **THEN** the UI exposes that more children exist and supports bounded navigation to subsequent pages.

#### Scenario: Recursive TUI frame
- **WHEN** multiple visible directories need nested treemap data in one frame
- **THEN** all nested reads share one SQLite deadline and a combined row/query budget; exhaustion leaves the parent blocks visible and identifies the truncation reason
- **AND** each frame and navigation rechecks the revision's actual scope and current metadata grant

### Requirement: Q-09 Bounded review-candidate preparation
For a positive target, CLI and MCP SHALL select review candidates through a deadline-bound database query without decoding an entire revision. The query SHALL preserve rebuildable evidence, protection/occupancy checks over ancestors and descendants, non-overlapping selections, and descending size priority. Budget exhaustion SHALL report an incomplete result and the unfulfilled target amount; it SHALL never imply that a partial review queue reaches the requested bytes.

#### Scenario: Unrelated corrupt node does not affect a candidate page
- **WHEN** a complete revision has a rebuildable directory and an unrelated node whose archived locator cannot be decoded
- **THEN** CLI and MCP return the candidate without loading the unrelated node.

#### Scenario: Protected descendant blocks a large candidate
- **WHEN** a rebuildable directory contains a protected or process-used descendant
- **THEN** the directory is excluded even if it is larger than every other candidate.

#### Scenario: Candidate preparation reaches its deadline
- **WHEN** the database query or selection walk exceeds its budget
- **THEN** the response reports `complete=false`, a truncation reason, selected bytes and the remaining target bytes.
