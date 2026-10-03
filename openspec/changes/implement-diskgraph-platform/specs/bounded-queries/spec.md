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

#### Scenario: Response accounting cannot wrap
- **WHEN** 库调用者提交的响应字节增量使累计 usize 溢出，或在可表示的精确上限后继续计费
- **THEN** 返回 ByteLimit 截断并保留原累计量，后续计费仍拒绝；debug 与 release 行为一致，不 panic、回绕或错误接受。可表示的精确上限本身仍可接受，不将溢出饱和成合法额度。

#### Scenario: Relation preparation and terminal authorization share one deadline
- **WHEN** related、explain、impact 或 candidates 的解析、授权、连接准备、读取或返回前授权等待耗尽请求期限
- **THEN** 全阶段使用同一个绝对协作期限，不在授权后重新计时；空结果、零目标和未完整覆盖同样不能成为晚到完整成功。能表示部分结果时保留已计费前缀并明确 Deadline，否则返回明确预算错误；同步 I/O、锁和调度不被描述为可抢占的严格墙钟保证。

#### Scenario: Response bytes include JSON escaping
- **WHEN** 关系 ID、名称、证据或诊断包含控制字符、引号、反斜杠或 Unicode
- **THEN** 查询字节预算按实际 JSON 编码计费，包含所承诺数据/诊断 envelope 的字段与分隔符，不用字符串原始长度近似转义成本；不能容纳最小诊断时返回预算错误。tools/call 的文本嵌套、HTTP/SSE 外层封装与传输上限仍分别校验，不把这些不同计量点混作一个保证。

#### Scenario: Raw fields are checked before owned decoding
- **WHEN** 有界关系或候选读取遇到超过剩余额度的原始证据/实体/选中节点字段，即使该字段解码后还会出现类型或格式错误
- **THEN** 在 Rust 拥有字符串、节点或证据及反序列化前执行原始字节门禁；预算内的真实解码错误继续传播，合法 NULL、旧 JSON 和未知大小语义保持。该门禁不声称限制 SQLite 内部页缓存、JSON 运算或整个查询 RSS。

#### Scenario: Tree and history share their preparation deadline
- **WHEN** 树、比较、changes 或 growth 的首次授权、归属解析、读连接准备或末段授权等待耗尽请求期限
- **THEN** 所有阶段继承解析前生成的同一个绝对期限，空结果和不兼容历史也不得晚到完整成功；保留旧可信 API 契约，有界前缀明确 Deadline，无法确认归属或容纳诊断则拒绝，不承诺抢占同步等待。

#### Scenario: History deadline fixtures distinguish preparation from a verified prefix
- **WHEN** 历史查询在连接/根元数据尚未完成时到期，或已经生成可验证报告后在最终授权阶段到期
- **THEN** 前者返回明确预算错误，不伪造报告；后者保留已比较条目并同时标记 deadline、complete=false 和 summary_is_partial=true。回归分别固定这两个阶段，不要求任意宿主在 1 ms 内完成准备，也不接受其他错误或晚到完整成功。

#### Scenario: Tree and comparison include the complete encoded report
- **WHEN** 树或历史路径含转义字符，或者最小报告头和诊断已超过正数响应额度
- **THEN** 预算覆盖实际 JSON 数据/报告头/诊断及承诺 envelope 的完整编码；不以路径长度近似，不容纳最小报告时明确预算错误。外层 truncated 与数据完成度保持一致；节点额度按双侧实际解码累计，超限 lookahead 不解码未返回节点。

#### Scenario: Both history revisions are authorized at return
- **WHEN** 历史数据生成或编码后，任一实际 revision 所属 scope 或主体的元数据 grant 被另一控制连接撤销
- **THEN** CLI/MCP 在返回前复核双侧实际归属和当前权限，拒绝数据；客户端 scope 不替代真实归属。第二侧最后能力回调撤销第一侧时，回调全部结束后的双方纯持久复检同样拒绝数据，不以逐侧检查成功代替整次末检。树 JSON/HTML 及通用授权 reader 同样执行末检，HTML 在写文件前复核；无持久策略的可信兼容 authorizer 回调也不能越过 scope 撤销。

#### Scenario: Tree unknown size retains numeric filter compatibility
- **WHEN** 已发布目录页含未知大小，且 minimum 过滤或节点预算限制展示范围
- **THEN** 继续沿用既有数值 subtree-size 过滤及精确聚合计数；返回未知节点明确 size_known=false，不将其数值解释为确定大小。稀疏或密集未知项的当前有界页不为补算任意阈值遍历所有兄弟，不引入未经验证的计数迁移。

#### Scenario: Terminal expiry cannot produce a usable plan
- **WHEN** 比较报告或同步计划编码后，末段授权等待耗尽整次期限
- **THEN** 普通报告若保留前缀，complete=false、外层 truncated 与已有 summary_is_partial 同时表示局部结果；同步计划直接拒绝，不输出可使用的 steps。

#### Scenario: Failure diagnostics include actual JSON escaping
- **WHEN** 历史读取中预算内字段发生格式错误，而错误诊断的实际转义编码超过默认响应额度
- **THEN** CLI JSON 错误 envelope 与 MCP 业务错误诊断用有界信息保留原业务错误码和退出码，不返回成功或将格式错误无声改成预算错误。JSON-RPC 请求 ID、文本嵌套和 HTTP/SSE 外包装仍分别受传输预算约束。

#### Scenario: Legacy native growth shares both snapshot budgets and terminal authorization
- **WHEN** 旧 UniFFI growth_json 比较两个 snapshot 的精确 locator，包含空匹配、不兼容、读取错误、响应转义或末段撤权/到期
- **THEN** 从解析/打开引擎前共用一个期限；两个 snapshot 元数据和节点共用原始字段/节点账本，先准入再解码，保持精确窄读，根与非根使用既有索引，200k行精确单节点读取含根探测少于500 VM步，同value不同locator类型不得误命中。完整成功 envelope 编码后成组复检双侧实际 revision 的 scope/grant；任一撤权、超限或到期拒绝数据。保留导出签名、schema、兼容时的 before/after 与字符串 delta_bytes，以及原不兼容/缺失的 null；失败诊断有界，不添加后台 owner 或重置期限。

#### Scenario: Native directory pages share raw admission and terminal request checks
- **WHEN** 旧 top_json/children_json 或 NativeService.children_json 读取目录页，遇到超大借用字段、坏 continuation、末段撤权/关闭/到期或 JSON 转义放大
- **THEN** 从打开引擎/首次准备前共用期限，snapshot头与当前页节点在分配/解码前累计准入，额度外 continuation 仅检查存在。旧 top/children 保留全部节点的尺寸/名称/ID排序、limit/offset和成功形态，预算不足明确失败；session继续仅列已知大小并返回精确unknown_size_count、实际页长推进的next_offset和准确截断原因。完整envelope编码后授权与取消仍须通过，错误诊断同样有界，不重置期限或引入共享请求状态。

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

#### Scenario: Relation response is revoked after its data read
- **WHEN** 数据读取完成后实际 revision 所属 scope 或请求主体的元数据授权被撤销
- **THEN** related、explain、impact 与 candidates 在返回数据前重新检查实时 scope 和授权交集，拒绝返回完整或部分数据；客户端 scope 提示不能替代真实 revision 归属，不增加共享请求状态或绕过旧授权路径。

#### Scenario: Traversed edges are cumulative across directions
- **WHEN** impact 读取同一实体的入边与出边，或遇到重复、自循环和不传播的关系
- **THEN** 实际解码边共用累计额度，不按方向重建预算，也不只对最终传播项计费；额外存在性探针不解码为完整关系，未读取部分明确报告截断，传播和排序语义保持兼容。

#### Scenario: Wide TUI directory
- **WHEN** a directory contains more children than one TUI page
- **THEN** the UI exposes that more children exist and supports bounded navigation to subsequent pages.

#### Scenario: Recursive TUI frame
- **WHEN** multiple visible directories need nested treemap data in one frame
- **THEN** all nested reads share one SQLite deadline and a combined row/query budget; exhaustion leaves the parent blocks visible and identifies the truncation reason
- **AND** each frame and navigation rechecks the revision's actual scope and current metadata grant

#### Scenario: Exact counts in a wide immutable directory
- **WHEN** tree or children requests a small page from a directory with hundreds of thousands of children, including unknown sizes
- **THEN** exact total, unknown and arbitrary minimum-size counts use published count/prefix indexes without traversing all siblings; known and unknown pages use matching ordered indexes
- **AND** count indexes are built atomically with the snapshot, backfilled transactionally on upgrade with a consistent pre-upgrade backup, and removed with the snapshot
- **AND** arbitrary minimum-size tree counts retain the existing numeric subtree-size semantics; the indexes add storage and publication work rather than promising a strict scan RSS cap

#### Scenario: Concurrent read-only CLI startup
- **WHEN** multiple trusted local CLI processes open an already initialized policy and repeat an existing grant
- **THEN** existing grants are not rewritten and unchanged graph schemas do not acquire a write transaction merely to drop an absent legacy index
- **AND** a genuine grant change remains durable and continues to enforce the current policy epoch

#### Scenario: Deep directory continuation
- **WHEN** MCP children continues a page using its returned v2 cursor
- **THEN** the next query seeks by actual subtree-bytes descending, name ascending and ID ascending, without walking the preceding siblings
- **AND** the cursor binds principal, actual scope/revision, parent, minimum-size filter, ordering and policy epoch; obsolete or mismatched cursors require a fresh query
- **AND** explicit offset inputs and next_offset outputs remain compatible, while a cursor resumes from its last returned item even when the byte budget shortens a page

#### Scenario: Deep search continuation
- **WHEN** search continues after a name/ID key in a large matching revision
- **THEN** it seeks through the ordered index rather than revisiting the preceding matches and retains Unicode lowercase substring semantics
- **AND** a probe outside the returned page is not decoded into an archived node

#### Scenario: Read-only query does not consume queued scans
- **WHEN** a one-shot CLI metadata query opens a store with a queued job
- **THEN** it does not start a background scanner; explicit synchronous indexing or the long-lived server runner owns execution
- **AND** CLI waiting on a failed/cancelled foreign job returns a nonzero incomplete outcome, never success with state failed

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

#### Scenario: Candidate evidence fits atomically within the remaining budget
- **WHEN** 一项候选的节点与必需证据超过剩余累计边数或原始/编码字节额度
- **THEN** 不提交缺证据候选，也不先增加 selected bytes 或减少 target 缺口；保留先前完整候选前缀并报告 EdgeLimit/ByteLimit 和精确缺口。保护/占用祖先与后代检查不能因缩短返回页而被省略，不能把实际两条证据当一项候选计作一条边。
