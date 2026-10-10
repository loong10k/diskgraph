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

#### Scenario: Independent reader preparation refuses expired or cancelled admission
- **WHEN** 独立只读连接开始准备前原绝对期限已过或取消标志已设置，或连接配置完成时上述条件成立
- **THEN** 不返回可用连接；已过期限返回预算错误，取消沿用 SQLite interrupted 语义。开始前失败不得先访问数据库路径；期限与取消同时成立时期限错误优先，准备中已有真实 SQLite 失败不被后检覆盖。检查后的并发失效仍须调用方及执行阶段复验；SQL 运行中的期限/取消仍由进度回调检查，不将入口检查描述为可抢占同步 I/O。

#### Scenario: Relation deadline fixtures distinguish owner preparation from authorized expiry
- **WHEN** 请求在准入真实 revision 归属前已到期，或完成归属准入后由实际授权回调耗尽原期限
- **THEN** 前者返回明确预算错误，不伪造已授权前缀；后者能容纳既有关系／影响／候选诊断时保留空 Deadline 截断及未完成目标。回归固定这两个阶段，不要求任意宿主在 1 ms 内完成连接／归属准备，不接受其他错误、额度重置或晚到完整成功。

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

#### Scenario: Historical namespaces use actual ownership rather than display equality
- **WHEN** two owned legacy revisions have equal displayed roots, volume and scan settings, but belong to distinct actual server/scope namespaces; the principal has metadata permission on both
- **THEN** changes/growth refuse numeric historical comparability and changes explain the namespace mismatch using compatible existing fields with additive diagnostics
- **AND** lossless registered roots and persisted ownership remain authoritative; old display equality cannot substitute for namespace equality, including distinct raw roots with colliding legacy display strings
- **AND** missing permissions, unbound old revisions and foreign-server ownership still return authorization failure before a normal incomparable result; terminal revocation, cancellation, deadlines and budgets remain enforced
- **AND** the separate authorized cross-root metadata comparison API continues to allow different scopes and is not globally restricted by the growth/changes compatibility rule

#### Scenario: Historical node sizes require observable facts
- **WHEN** either matched node has unknown size or a recorded read error, even when an imported snapshot header claims complete coverage
- **THEN** growth returns no numeric delta and paired size-change statistics do not treat stored placeholder bytes as observed sizes; comparison reports the existing unknown-size outcome instead of SameMetadata, including directories with equal aggregate counts
- **AND** owned comparison rows retain their existing fields but use null for unavailable sizes; actual decoding, budget, timeout and authorization failures remain errors rather than becoming normal unknown data

#### Scenario: Type replacement is not size growth
- **WHEN** a locator changes between file, directory, symlink or another recorded kind
- **THEN** growth refuses a delta and changes do not count the replacement as numeric size growth; comparable known/readable nodes of different kinds produce the existing path/type difference verdict
- **AND** known zero and negative growth remain real answers; neither file identity equality nor matching roots is added as a requirement for the separate cross-root metadata comparison API

#### Scenario: Coverage diagnostics cannot be overridden by a complete flag
- **WHEN** 原生库消费者直接构造快照，或历史数据的 complete=true 与 unreadable_nodes>0 或 depth_limited=true 同时存在
- **THEN** growth 不提供可信数值，changes 返回 incomplete_coverage 而不推断移除或尺寸变化，审阅候选不把此覆盖提升为可确定重建范围
- **AND** 公开 Store 发布仍拒绝矛盾覆盖并整体回滚；合法完整及显式 partial 的既有 JSON 字段、未知大小和不可比原因保持兼容。

#### Scenario: Unknown results retain terminal authorization and budgets
- **WHEN** Engine or legacy FFI growth has produced an unknown result
- **THEN** both revisions still undergo the existing terminal authorization, cancellation, deadline and response-budget checks; no early unknown return bypasses those checks

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
The system SHALL decode only the requested page or bounded tree nodes, use independent SQLite read connections with deadlines, preserve Rust Unicode lowercase substring search, and bind v2 keyset cursors to principal, scope, revision, filters, actual ordering and policy version.

#### Scenario: Enforce every constraint of Q-08
- **WHEN** the implementation is built, modified, or used
- **THEN** Explicit offsets SHALL remain supported. Legacy cursors SHALL be rejected and require a fresh query. Ordered history merge SHALL retain no more than the output budget plus current iterator entries; truncated statistics SHALL be labelled partial.

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

#### Scenario: TUI deadline truncation reaches the actual backend
- **WHEN** 整帧嵌套读取耗尽共同期限，但已缓存父块可以形成明确标记的部分帧
- **THEN** 实际终端后端仍显示父块及 deadline 截断提示，不因预算错误直接退出或丢弃该部分帧；不得将晚到完整结果表示为成功。
- **AND** 提交缓冲前再次核验真实 revision 归属、scope 和元数据权限；撤权、过期身份及实际存储错误仍阻止完整或部分数据提交。不得放宽通用授权 reader 的期限契约，后续查询不复用已经耗尽的读取额度。

#### Scenario: TUI preparation excludes unnecessary owned metadata
- **WHEN** 合法快照的节点含超出展示预算的大型 reclaim_hint 或其他不用于导航绘制的元数据
- **THEN** 页面与整帧读取使用必要字段投影或读取前原始字段准入，不先拥有这些未使用字段再计算展示成本。必需的名称、路径及节点读取按整次预算计费，真实准备成本与展示成本分别报告；不把有限行数或256KiB展示额度描述为完整原始输入或RSS上限。

#### Scenario: Revision target preparation shares the actual query ledger
- **WHEN** 合法已发布 revision 的必需 snapshot ID 或实际归属字段超过 TUI 导航、整帧、历史、related、explain、impact、candidates 或 tree 请求的剩余原始字段额度
- **THEN** 在拥有该字段之前执行借用字段准入；归属、目标、节点及后续读取沿用从首次准备前建立的同一账本和期限，不能在消费者中重建额度。不会为不需要的 revision 元数据分配完整记录。
- **AND** 初次目标准备失败返回明确预算错误，不能转换成可提交的缓存部分画布；实际终端后端没有提交该帧。
- **AND** 关系、候选和树复用已准入目标，不为构造证据 reader 再拥有完整 revision 或重复读取目标；候选快照覆盖、实体、邻接和树节点沿用剩余原始余额。已解析并授权真实归属之后的目标预算失败仍执行实时末段授权，不因错误路径跳过撤权检查。
- **AND** 无法在期限内取得目标时，树不得伪造根；关系/影响或候选如能返回既有明确 Deadline 空前缀，必须保持未观测诊断及完整目标缺口，不能读取已经到期的头来补齐成功。可信内部有界入口仍执行目标准入，其原授权责任不被更改。

#### Scenario: Both historical targets consume one preparation budget
- **WHEN** 单侧历史头可容纳，但双侧必需头与后续快照读取累计超出请求额度
- **THEN** comparison、changes 和 growth 共用累计准入，不能按侧或进入有序合并时重新获得额度；两侧均已解析并授权后发生的准备或编码失败仍执行双侧实时末段授权，不返回撤权后的完整或部分数据。
- **AND** 测试从真实公开请求起点计量 Rust 分配并覆盖合法大型 snapshot ID，另有普通头和足够额度的真实成功对照；此计量不代表 SQLite C 分配、文件系统 I/O 或 RSS 上限。

#### Scenario: TUI initial and navigation authorization remain bounded
- **WHEN** 首次帧授权或导航读取遇到由另一线程持有的控制库锁
- **THEN** 专用 TUI 路径非阻塞拒绝竞争，初次归属与权限检查及导航准备沿用原请求期限；不得等待锁释放后把已经迟到的初次授权转换为可提交部分帧。
- **AND** 导航只在实时终检成功且完整结果仍在原期限内时返回 Layer，不复用画布截断的晚到提交通路；通用可信 reader 的兼容契约保持不变。

#### Scenario: Live scope authorization excludes unused registered metadata
- **WHEN** 有界展示/导航请求执行首末授权，或关系请求执行末段授权；合法注册 scope 的展示根或卷说明含大型字段，但已发布 revision 归属和必需导航字段很小
- **THEN** 首次及末段授权只读取所需的实时撤销标志，不拥有完整 ScopeRecord；原控制锁、能力回调前后检查顺序、实时 grant 查询和共同期限保持不变。缺失 scope 或所需授权字段格式错误仍传播，不缓存授权事实。
- **AND** 同一真实 display 请求在原50ms期限和256KiB读取预算下完成消费者并经过两次实际允许决定，再验证整个调用的 Rust requested allocation 小于512KiB；超时、拒权或未进入消费者不能当作分配门禁通过。此计量不代表 SQLite C 分配、I/O 或 RSS。
- **AND** 独立控制连接在末段能力回调撤销 scope 或 grant 时，完整结果仍返回 PermissionDenied。公开 scope() 完整记录契约不变；仅授权投影不再解码其未使用的根/展示/卷字段，不能宣称这些无关字段的损坏错误仍在授权路径被发现。

#### Scenario: Exact counts in a wide immutable directory
- **WHEN** tree or children requests a small page from a directory with hundreds of thousands of children, including unknown sizes
- **THEN** exact total, unknown and arbitrary minimum-size counts use published count/prefix indexes without traversing all siblings; known and unknown pages use matching ordered indexes
- **AND** count indexes are built atomically with the snapshot, backfilled transactionally on upgrade with a consistent pre-upgrade backup, and removed with the snapshot
- **AND** arbitrary minimum-size tree counts retain the existing numeric subtree-size semantics; the indexes add storage and publication work rather than promising a strict scan RSS cap

#### Scenario: Encoded prefixes retain bounded terminal authorization after data expiry
- **WHEN** an adapter has prepared a bounded prefix and its original data deadline expires before response finalization
- **THEN** finalization observes actual revision ownership and current scope/grants with fixed 250 ms database authorization-only windows before and after the capability callback, using an independent reader and refusing control-lock contention without waiting
- **AND** that reader is never exposed to a data consumer; the original data deadline is not renewed. Successful observation returns expired status, permitting only the existing explicit incomplete/deadline response. Revocation, ownership mismatch or failed authorization observation refuses all data. Capability callbacks have a fixed cooperative 50 ms acceptance window; fresh SQL after callbacks still checks revocation before rejecting a late permit. Adapters retain their existing cancellation checks before and after finalization; this method does not introduce a cancellation input.

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

#### Scenario: Candidate snapshot preparation shares raw admission
- **WHEN** 候选查询读取合法持久快照头，包括体积较大的 volume/provider 标识
- **THEN** 快照头在拥有或 JSON 解码前按同一次原始字段账本准入，后续候选节点及证据共用剩余额度；初始快照头原始字节额度不足时返回明确预算错误，不伪造覆盖状态、空完整队列或已达目标。可准入的头保留原覆盖、优先级和保护/占用语义。

#### Scenario: Candidate deadline preserves an explicitly unobserved header
- **WHEN** 候选读取在取得可信快照覆盖前已耗尽原期限
- **THEN** 不再读取或解码快照头，保留既有空候选 Deadline 截断及完整目标缺口；coverage_observed=false 明确表示覆盖尚未确认，保守 coverage_complete=false 不能解释为已观测到覆盖缺口。成功取得头后 coverage_observed=true，CLI/MCP/FFI 均传递该新增诊断；结果仍经过原响应预算及实时授权末检，真实格式错误、缺索引或非期限预算失败不能被吞成 Deadline。

#### Scenario: Candidate evidence fits atomically within the remaining budget
- **WHEN** 一项候选的节点与必需证据超过剩余累计边数或原始/编码字节额度
- **THEN** 不提交缺证据候选，也不先增加 selected bytes 或减少 target 缺口；保留先前完整候选前缀并报告 EdgeLimit/ByteLimit 和精确缺口。保护/占用祖先与后代检查不能因缩短返回页而被省略，不能把实际两条证据当一项候选计作一条边。


#### Scenario: TUI initial authorization expiry rejects cached data
- **WHEN** 普通合法 revision 的首次真实授权回调延迟超过整帧原50ms期限，或者超大必需头在同一账本内被拒绝
- **THEN** 首次准备返回明确预算错误，不进入 paint、不向实际终端提交旧缓存；授权回调后的阶段不得重新获得时间窗口。超大头仍在拥有前受原始字节准入约束，Rust累计分配门禁保持不变。
- **AND** 功能回归使用真实策略的延迟委托固定初次到期阶段，不把任意宿主的调度墙钟作为硬实时保证；记录实际调用耗时，生产50ms额度不增加，普通成功对照和已有绘制阶段到期/末段撤权测试保持独立。

#### Scenario: Control configuration preparation consumes the original read window
- **WHEN** real SQLite configuration preparation completes after the original control read deadline
- **THEN** the read consumer SHALL NOT be called at that observed expired admission boundary
- **AND** the result SHALL be BudgetExceeded, with the connection's actual prior busy timeout and progress configuration restored
- **AND** a timely admitted consumer's original error or panic SHALL retain its existing propagation and restoration semantics

#### Scenario: Terminal relation and history control contention

- **WHEN** a relation/tree or history query reaches terminal authorization while another thread holds the actual control store guard
- **THEN** it refuses with budget_exceeded without waiting for that holder to release the guard, and returns no complete or partial payload without terminal authorization.
- **AND** the original query deadline is not renewed; uncontended paths retain live grant, revocation, and actual revision ownership checks before and after encoding.

#### Scenario: Late terminal capability still observes cross-side denial
- **WHEN** 关系或历史末段能力回调在原 50ms 能力观察窗口之后返回允许
- **THEN** 完成必要的各侧持久授权和新鲜 revision 归属观察后拒绝预算，不提交完整或部分编码结果。
- **AND** 后侧回调撤销前侧、其他侧已撤权或回调期间 revision 被隔离时，实际拒权优先于迟到允许。控制 SQL 分阶段设置执行期限，能力回调不位于 SQL progress guard 内，不能嵌套覆盖已有 guard；同步回调不承诺硬抢占，原数据期限不得刷新。

#### Scenario: Ordinary revision readers tolerate brief terminal control contention
- **WHEN** a CLI/MCP authorized narrow reader finishes its capability callback while another request briefly owns the shared control connection
- **THEN** terminal lock acquisition and its SQL observation share the fixed 250 ms database observation window; waiting for the lock is additionally capped by the original request deadline, and brief contention alone does not fail the request
- **AND** an immediately available control guard still observes explicit revocation in the existing terminal SQL window after the data deadline; this negative observation never extends data work or permits a successful result after the original deadline
- **AND** a lock held past that window returns BudgetExceeded, withdrawal and live authorization checks remain mandatory, and dedicated TUI nonblocking admission is unchanged

#### Scenario: 响应末检已知 revision 隔离优先于未知授权代次
- **WHEN** 能力回调期间其他主体授权改变代次，且当前 revision 已实际持久隔离
- **THEN** 响应末检在原固定观察窗口内用新鲜归属投影返回 PermissionDenied
- **AND** 原归属审计记录保留；若 revision 仍可授权则保留 Conflict，不刷新期限或返回受保护数据

#### Scenario: Database terminal observation is independent of the capability window
- **WHEN** terminal authorization encounters 100ms of actual control-lock contention within an unexpired original request deadline
- **THEN** ordinary revision readers may complete within the fixed 250ms database window; lock acquisition and SQL share that same absolute window
- **AND** contention exceeding 250ms refuses results, and a shorter original request deadline caps waiting without renewal
- **AND** capability callbacks retain their independent cooperative 50ms acceptance window; late permits, revocation, quarantine, cancellation and expiry still refuse protected data
