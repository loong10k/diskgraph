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

#### Scenario: Evidence batches remain revision isolated
- **WHEN** 同一文件快照产生多个采集批次，或新批次尚未发布
- **THEN** 对外关系、解释、影响与候选查询仅观察指定revision的active断言；dependency_only批次只用于解析引用来源，未绑定批次和歧义旧数据不得进入结果。可信snapshot级Store入口保留兼容。

#### Scenario: Atomic collector publication
- **WHEN** 新采集批次发布，或首次扫描同时发布确定性项目证据
- **THEN** run、实体、证据、关系、归属、批次绑定和新revision在同一图库事务提交；失败没有可见半成品，旧revision的绑定不可变。跨snapshot、未知role、伪造来源和冲突实体拒绝整批。

#### Scenario: Safe legacy membership migration
- **WHEN** 旧数据库升级到按revision选择采集批次的结构
- **THEN** 使用一致性备份，仅在实体来源或边引用证据可唯一确认run时回填成员关系；歧义条目对外不可见并提供重新采集诊断，不把同snapshot所有批次自动加入所有revision。

#### Scenario: Revision-scoped candidate blockers
- **WHEN** 一个文件快照被多个revision复用，某revision选择的active批次包含进程占用或保护关系
- **THEN** Engine、CLI/MCP与原生FFI候选查询按本请求捕获的revision排除该资源及祖先/后代；其他revision和dependency_only断言不污染本次选择。保留旧静态重建条件，正向观察即使过期也不自动变为无占用证明，结果始终仅供审阅。

#### Scenario: Legacy revision integrity
- **WHEN** 旧占用或保护观察缺少实际快照中的有效节点定位，或历史版本遗漏 active 边端点的原始来源、选择非法 role 或跨快照运行
- **THEN** 迁移把该版本标记为 incomplete，候选及关系入口要求重采；不得猜测节点或自动追加上游。来源可确认的其他版本保持可查询，partial 或过期正观察仍阻止对应候选。

#### Scenario: Ambiguous evidence does not disable unrelated metadata
- **WHEN** 旧证据成员来源无法确认
- **THEN** 关系、解释、影响及候选入口明确要求重采或重新索引；同一快照的文件树等不依赖证据的元数据查询继续经过原有权限与预算门禁。

#### Scenario: Writer generation and sealed selection
- **WHEN** 一个v9写进程已打开图库，另一个进程升级到v10，或调用者尝试改写已发布版本的批次选择
- **THEN** 数据库拒绝旧代次创建collector/revision及已封存选择的追加、更新、直接删除和重新解封；显式历史回收通过删除父revision级联删除其选择，并保持失败回滚。

#### Scenario: Complete selected provenance and recollection recovery
- **WHEN** 新版本选择已有采集运行，或在存在歧义旧证据的文件快照上重新采集
- **THEN** 封存前验证全部所选运行的原始实体来源闭包；新版本不能把有歧义的旧运行提升为完整。只选择已确认的新批次可以恢复，同快照旧版本仍保持incomplete。

#### Scenario: Stale collector publication
- **WHEN** 基于旧版本的采集尚未发布，另一次扫描或采集已经更新latest
- **THEN** 旧基线发布在事务内拒绝，不把最新文件树或批次选择回退；调用者须从最新版本重新准备。

#### Scenario: Historical same-entity page cost
- **WHEN** 同一实体累积大量inactive历史关系，而所选版本只含少量active关系
- **THEN** 有界查询只按有效批次邻接索引定位并合并keyset，准备阶段的运行与关系键纳入预算；2万/20万历史不会被逐条扫描。保留双向去重、稳定顺序和不解码lookahead。

#### Scenario: Deadline expires after scan staging
- **WHEN** 扫描已完成暂存或项目采集，但提交前的单调时钟期限已经耗尽
- **THEN** 在实际fencing事务内、写入图库前拒绝发布并清理本代次staging；不得留下snapshot、revision或collector半成品。本检查是提交前协作门禁，不承诺事务执行中严格抢占。

### Requirement: EV-06 Application and process coverage
应用/进程采集 SHALL 报告方法、权限范围与 unsupported/denied/partial 状态；PID 应附启动上下文，应用标识应区分安装实例。

#### Scenario: No visible processes
- **WHEN** 低权限观察返回空结果
- **THEN** 仍报告受限覆盖，不能断言没有使用者。

#### Scenario: Unverified external probe coverage
- **WHEN** 外部进程探针成功返回可见句柄，但未验证系统权限范围或 PID 启动上下文
- **THEN** 保留正向观察，覆盖为 partial；退出码 0 或空结果退出码 1 均不能独自证明完整覆盖。

#### Scenario: Malformed process records
- **WHEN** 探针输出包含非法字段、无效 PID、缺少进程上下文或未完整终止的记录
- **THEN** 返回不可确认的诊断，不 panic，不将解析失败解释为完整空样本。

#### Scenario: Native path identity in process evidence
- **WHEN** 两条原生路径仅在有损 UTF-8 显示转换后相同
- **THEN** 不将其当作同一采样对象；只有可确认的字段才按原生路径键匹配，无法保真匹配的编码或平台明确保持 unknown。

#### Scenario: Ambiguous display suffix
- **WHEN** 探针路径文本带有可能也是合法文件名的 deleted 标记
- **THEN** 没有可靠身份依据时不删后缀猜测另一个对象；无法确认的观察保持 unknown，不据此报告未占用。

#### Scenario: Escaped display path identity
- **WHEN** NUL 字段中的路径显示包含转义或无法确认的名称编码，可能等于另一个对象的原生名称
- **THEN** 不猜测解码、不把显示文本当原始身份；保持 partial/unknown，保留其他无歧义的正向观察。

#### Scenario: Repeated process handles do not multiply command storage
- **WHEN** 有界探针输出在同一进程上下文重复报告大量匹配文件，或重复出现相同 PID/命令上下文
- **THEN** 在拥有命令字符串前去重，保留原有按 PID/命令排序的唯一持有者；暂存字符串总量不得随重复文件数乘以命令长度放大。异常记录、歧义路径和未验证身份的覆盖语义保持不变。
