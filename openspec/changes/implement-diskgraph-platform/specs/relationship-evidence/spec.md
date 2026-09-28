## Purpose

定义目录资源与应用、项目、进程、重建规则和保护策略之间可解释的关系，保留来源、时间、冲突及覆盖状态，使智能体能够区分观察事实、推导结果与启发式判断。

## ADDED Requirements

### Requirement: EV-01 Typed entities and edges
系统 SHALL 提供 Resource、Project、Application、Process、BuildRecipe、ProtectionPolicy 的有类型观察及合法关系端点；包含关系和归属关系不得被混为运行依赖。

#### Scenario: Shared cache
- **WHEN** 同一缓存存在两个有依据的项目归属
- **THEN** 保留两个所有者，不将空间重复计为两份。

### Requirement: EV-02 Provenance and conflict
每项关系 SHALL 关联采集器/规则版本、时间、输入依据、observed/derived/heuristic/user_policy 类型和支持或反驳证据；冲突不得静默覆盖，可信度分数不得当作统计概率或删除许可。

#### Scenario: Name match conflicts
- **WHEN** 名称匹配与容器元数据指向不同应用
- **THEN** explain 展示双方依据、类型与冲突。

### Requirement: EV-03 Independent freshness
系统 SHALL 分别追踪文件观察、进程观察、规则和保护策略的新鲜度；过期占用不能变为未占用，暂时读取失败不能撤销保护。

#### Scenario: Process evidence expired
- **WHEN** 进程采集已过有效期
- **THEN** 风险状态为 stale/unknown，不把构建目录提升为可安全删除。

### Requirement: EV-04 Deterministic project evidence
系统 SHALL 有界解析 Cargo/package 等声明和支持的构建配置，不执行项目脚本来发现归属；对 workspace、嵌套项目、自定义与共享输出分别举证。

#### Scenario: Unrelated target folder
- **WHEN** 普通目录名为 target 但没有支持的项目证据
- **THEN** 只能保留类别提示，不产生确定归属或可重建资格。

#### Scenario: Overridden output
- **WHEN** 输出位置来自无法观察的环境或命令行覆盖
- **THEN** 报告归属不确定，不假定默认布局正确。

### Requirement: EV-05 Revision selection
查询 SHALL 固定文件快照与证据批次组合；新进程或应用观察发布新 revision，保留旧解释并包含被引用的上游证据。

#### Scenario: Process refresh
- **WHEN** 文件快照不变而进程信息更新
- **THEN** 新旧 revision 可分别查询，不原地改写旧结果。

### Requirement: EV-06 Application and process coverage
应用/进程采集 SHALL 报告方法、权限范围与 unsupported/denied/partial 状态；PID 应附启动上下文，应用标识应区分安装实例。

#### Scenario: No visible processes
- **WHEN** 低权限观察返回空结果
- **THEN** 仍报告受限覆盖，不能断言没有使用者。
