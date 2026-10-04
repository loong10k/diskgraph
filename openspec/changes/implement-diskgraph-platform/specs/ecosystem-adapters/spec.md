## Purpose

将 Git、进程观察以及 Cargo、Docker 等专业生态能力作为可选适配器接入，在缺少工具或权限时保持基础索引可用，并防止把业务对象存储目录误当作普通缓存直接删除。

## ADDED Requirements

### Requirement: EC-01 Native baseline without shell tools
基础目录查询、扫描和文件操作 SHALL 不要求安装 ls/find/du/stat/cat/mv/cp/rm 或 disktree 应用；外部专业工具缺失只影响其对应能力。

#### Scenario: Minimal server image
- **WHEN** 服务器未安装常见 Shell 文件工具但支持所需系统 API
- **THEN** 基础索引和查询仍工作，能力报告不虚构专业适配器可用。

### Requirement: EC-02 Git and process evidence
可选证据适配器 SHALL 报告 Git 修改、stash、相对已知跟踪引用的提交差异及进程可见覆盖；Git 检查不默认联网，无法证明的远端或占用状态保留 unknown。

#### Scenario: No upstream or low privilege
- **WHEN** 仓库无跟踪分支或进程信息受权限限制
- **THEN** 不得声称所有提交已推送或目录无人使用。

#### Scenario: Explicit Git observation is a durable authorized job
- **WHEN** CLI 或 MCP 对明确的 scope、base revision 和目录 node 请求 Git collector，且真实请求主体的 MetadataRead、IndexWrite、ContentRead 上限与实时数据库授权均允许
- **THEN** 验证 revision 的实际本机 server/scope ownership 和同 snapshot 节点的无损原生目录定位后返回持久 job_id；队列保存固定目标与服务端验证的请求权限上下文，不把 Git 任务合并为旧扫描任务。断开连接及重开数据库仍可查询同一任务、主体与范围，入队本身不发布新的 revision。
- **AND** 只有运行、来源复核及发布末检成功才发布 Git CollectorRun 和最小化证据；新旧 revision 共享文件 snapshot，旧解释保持不变。dirty、stash 与本地已知跟踪引用差分可由真实 Git 正控核对，无 upstream 仍为 unknown，不转换为已验证远端或可删除结论。仅参数发现或成功入队不算完成该场景的执行与发布验收。

#### Scenario: Git request permissions cannot be supplied by the client
- **WHEN** Git 请求缺少 ContentRead 的真实 token ceiling 或实时数据库 grant，或请求 scope 与 revision 的实际 ownership 不匹配
- **THEN** 源输入访问及入队前返回稳定 permission_denied，不产生额外任务、采集批次或 revision；数据库允许不能补足请求上限，请求声明也不能补足实时授权。
- **AND** principal、issuer、ceiling 与 expiry 来自可信适配器已经验证的上下文，不从 collector 参数读取，不持久化 bearer；后续执行、取消、撤权、到期及恢复遵循 SC-06 和既有任务 fencing 契约，不能用本机 runner 身份补足。
- **AND** 共享任务调度遵循 RT-02 的候选页门禁：每轮最多 64 项逐项检查和认领，不能为新增请求身份校验引入全队列权限 JSON 解码或单事务扫描；这不替代 Git 捕获本身的输入、输出及期限预算。

### Requirement: EC-03 Domain cleanup uses domain semantics
Cargo/Docker 等执行 SHALL 使用版本化、允许列表化的专用操作计划；显示精确项目/生态对象、范围及不可恢复性，不通过直接删除数据库、Docker VM 磁盘或卷目录替代专业接口。

#### Scenario: Docker volume cleanup
- **WHEN** 用户明确选定卷清理计划
- **THEN** 重验选定卷身份与使用状态，只执行被批准对象；不扩大为全量 prune。

#### Scenario: Cargo absent
- **WHEN** Rust 项目清理适配器无法找到合格 Cargo
- **THEN** 返回 unavailable，不退化为 rm -rf。

### Requirement: EC-04 Restricted subprocess boundary
适配器 SHALL 固定并验证程序来源、参数结构、工作目录、环境、超时和输出上限，不接受模型任意 Shell、可执行路径或无限重试；项目配置/钩子带来的执行风险必须在能力和计划中声明。

#### Scenario: Product Git collection uses only the scoped private capture
- **WHEN** 持久 Git 任务实际执行固定 base revision/node 的采样
- **THEN** 从已注册的原生范围根及该节点的无损定位建立 scoped capture，禁止向父目录另找仓库或退回可信旧实时工作树模式；客户端不能指定程序、argv、路径、Shell 或网络开关。准备、捕获、全部固定子命令、终检及清理共用同一任务期限、取消状态、累计输入/输出及私有分配预算。
- **AND** 在已经到达捕获、执行及发布阶段的真实同步点撤权、取消、到期或失去 lease/fence 后，拒绝完整成功及原子发布；不能用前置参数失败代替这些末段回归。来源、版本、覆盖和有界诊断可查询，正文、patch、stash 消息、原始命令输出与配置秘密不得泄入元数据结果。库层捕获通过不代替持久任务与发布验收。

#### Scenario: Argument injection
- **WHEN** 文件名含选项前缀或 shell 元字符
- **THEN** 作为校验后的独立数据参数处理或拒绝，不成为命令语句或额外选项。

#### Scenario: Shared sample execution budget
- **WHEN** 一次证据采样执行多个子命令或 stdout/stderr 持续输出
- **THEN** 全部命令与两条管道共用绝对期限、累计字节预算与取消状态；失败停止后续工作，不返回完整成功样本。

#### Scenario: Shared budget across multiple observations
- **WHEN** 同一证据任务连续采样多个 Git 项目或进程路径集合
- **THEN** 使用独占且不可克隆的会话共享从创建时起的绝对期限、累计 stdout/stderr/stash 输出和取消状态；调用间耗时也计入期限。Git 元数据字节和条目累计扣费，成功清理后的真实余额可继续使用，不为下一目标补充额度。

#### Scenario: Native Git input reads respect remaining bytes
- **WHEN** 捕获 Git 元数据或读取 stash 日志时，普通文件已超过剩余原始输入额度，或文件在初始长度观察后增长
- **THEN** 已知超限在读取正文前拒绝；每次原生读取的缓冲区不超过剩余额度和初始长度的未读部分。长度、版本、取消或期限复核失败不返回完整元数据或局部 stash 数；不能先读取额外一块后才发现额度耗尽。
- **AND** 精确额度、空文件和短读取保留原有语义，额度内实际读取才扣费；此约束用于普通文件输入，不改变子进程管道的有界 EOF 观察与清理契约。

#### Scenario: Observation failure closes the session
- **WHEN** 会话中的 Git 准备、执行、解释、复核或清理失败，或进程解释返回 unobservable
- **THEN** 锁存首次完整错误，后续 Git/进程调用不启动程序，空路径请求也不得绕过失败；正常 partial 正向观察可继续但不提升为 full。旧独立 bounded 函数保留各自新建采样预算的兼容语义，会话不替代请求授权。

#### Scenario: Inherited pipes and cleanup
- **WHEN** 子进程退出但后代持有管道，或执行、读取、等待、取消失败
- **THEN** 有界读取仍检查期限，释放本次采样的进程与句柄；平台无法可靠建立清理边界时拒绝，不影响其他并发采样。

#### Scenario: Windows creation and I/O ownership
- **WHEN** Windows 启动证据探针，或取消未完成的异步管道读取
- **THEN** 创建时通过 Job 与限定继承句柄建立清理边界，缺少能力时拒绝；取消请求之后继续持有读取缓冲及 OVERLAPPED，确认最终完成后才释放，不声明内核 I/O 的严格墙钟上限。

#### Scenario: Exact output limit and terminal checks
- **WHEN** 两管道和多个命令累计恰好达到输出上限，或进程退出后管道未 EOF
- **THEN** 继续以固定小缓冲检查 EOF 与期限/取消，只有全部管道完整结束且终态预算仍有效才成功；额外字节、失败或晚到结果不能成为完整证据。

#### Scenario: Abnormal exit and cleanup diagnostics
- **WHEN** 探针因信号或异常状态退出，或主采样失败后清理也失败
- **THEN** 停止整次采样及后续命令，不降级成缺少引用；返回主错误并保留次级清理诊断，Drop 只能兜底。

#### Scenario: Retained leader and Windows cleanup observation
- **WHEN** Unix leader 离开原组，或 Windows 已终止进程仍由外部句柄引用
- **THEN** Unix 只终止仍拥有的 leader PID，不跟随新组；Windows 清理计数采用有限观察期，不能因外部引用无限等待或把未确认清理表示为完整成功，未完成自有 I/O 的安全释放仍按原生完成契约执行。

#### Scenario: Git configuration execution
- **WHEN** 仓库配置提供 fsmonitor、clean/process filter 或其他外部程序能力
- **THEN** 只读 Git 采样必须使用不执行这些程序的受限配置或明确拒绝；配置检查后的竞态不得恢复外部执行，不能仅按命令名称声称离线。

#### Scenario: Git preparation uses the same boundary
- **WHEN** 为采样定位元数据、解析配置或检查 index 依赖
- **THEN** 准备阶段同样遵守整次期限、取消及资源预算，从首次工具调用关闭 pager、继承 trace/loader 配置、懒取与可选写入；原 index 不得交给可能执行 fsmonitor 或刷新 shared index 的准备命令。复制的配置作为数据解析，不调用原配置程序。

#### Scenario: Git executable stays fixed across working directories
- **WHEN** PATH 包含当前目录、空项或相对目录，准备和采样使用不同工作目录
- **THEN** 只从绝对 PATH 目录解析受信工具一次并固定绝对程序路径；后续不能因进入工作树而运行仓库内同名程序，没有可用受信程序时明确拒绝。

#### Scenario: Native repository shadow acceptance is isolated from compiler artifacts
- **WHEN** 原生验收通过编译同名程序检验真实工作目录搜索与公共采样隔离
- **THEN** 编译输出位于工作树外的独占夹具目录，只将指定可执行文件放入工作树；固定绝对 Git 的真实 NUL status 必须先证明唯一预期未跟踪项。实际同名程序执行 marker、公共采样不执行 marker、精确 dirty 数及源元数据不变均须成立，不能以放宽数量、跳过平台或假设编译只有一个产物替代验收。

#### Scenario: Native Windows paths remain distinct from tool representation
- **WHEN** 受控私有目录或已验证工作树在 Windows 使用 verbatim drive 路径，而 Git 的配置、环境、参数或 alternates 不接受该前缀
- **THEN** 原生身份检查继续使用原路径；工具路径只对无歧义本地 drive 名称进行精确适配，不能 canonicalize 元数据叶、关闭 protectNTFS、继承宿主 Git 配置或将设备/UNC/点步/ADS/不可表示路径规范化成其他对象。实际 Windows 夹具须比较同一文件的原生身份及工具读写行为，普通仓库采样和 linked-worktree 回归仍须通过。

#### Scenario: Unrepresentable shell search entries cannot block the fixed tool
- **WHEN** 宿主 PATH 同时包含可用的受信工具目录及不能安全表示的其他搜索项
- **THEN** Git 绝对程序仍只解析一次；子进程 shell PATH 只保留可安全表示的绝对目录，逐项检查取消及期限，不把不支持的搜索项改写为其他目录。缺少必要 shell 或程序时传播真实启动失败；工作树、私有元数据及 SystemRoot 等实际执行路径仍须严格校验，不能以过滤搜索项代替身份或资源检查。

#### Scenario: Captured directories do not accumulate live handles
- **WHEN** 有界元数据视图捕获多个目录并保留来源复核记录
- **THEN** 捕获与枚举期间持有安全路径解析所需句柄，记录身份、版本、祖先身份和名单后释放；最终复核重新安全打开并比较，不为每个历史记录长期占用一组目录句柄。

#### Scenario: Host configuration is bounded before parsing
- **WHEN** 采样需要保留受信 Git 的宿主系统配置，而其原始文件超预算、是特殊文件或引用 include
- **THEN** 只使用已核验且不读写目标配置的固定路径发现接口；发现后先原生有界捕获，再解析私有副本，超限或特殊文件不能先交给 Git 解析。发现接口若依赖固定可信 shell，能力与测试必须明确该条件，宿主路径别名在终态重新发现，不能称为原子快照。

#### Scenario: Private allocation and volume headroom are checked separately
- **WHEN** 原始输入额度仍足够但许多短文件的原生报告分配超限，或临时卷可用空间不足或不可观测
- **THEN** 在准备创建和后续写入门禁拒绝；以对象原生报告的分配和卷可用空间分别计量，不能把逻辑内容字节当实际分配。只允许覆盖本 owner 登记的普通文件，末段复核未知项或分配变化并保留显式清理诊断；不宣称卷空间 reservation 或文件系统全局元数据精确归属。

#### Scenario: Private Git view preserves status semantics
- **WHEN** 私有元数据视图关闭外部程序或遇到属性、忽略、行尾、index、子模块或引用后端的特殊语义
- **THEN** 保留能可靠验证的状态语义；不能可靠保持的条件明确拒绝，不能删去 filter、忽略子模块或默认格式后返回正常 clean/dirty。原仓库配置及元数据的后续替换不得重新进入私有执行配置。

#### Scenario: Object database input is closed and bounded
- **WHEN** 源对象目录包含递归 alternates、promisor、链接或超预算对象，或在视图准备后新增外部对象路径
- **THEN** 在把对象交给 Git 前以原生 no-follow 捕获并按同一次期限、取消、累计原始字节及条目预算复制安全普通 loose 对象和配对 pack/index；Git 只读取私有扁平对象库，不把源 objects 或 info/alternates 挂入私有 alternates。既有 alternates/promisor 明确拒绝；准备后的外部路径不能成为 Git 输入，终态拒绝源捕获集合变化。复制不得用会改变源 nlink/ctime 的硬链接，不读未计费的巨大 alternates 文件，也不能以最终报错代替已发生的外部读取隔离。

#### Scenario: Scoped Git source capture precedes subprocess access
- **WHEN** 新增受范围约束的 Git 采样模式捕获已明确定位的仓库根、工作树、元数据及对象库
- **THEN** 所有源读取、枚举与终态复核均从同一保留的原生范围根句柄解析原始相对组件；不先读取或 canonicalize 后再检验范围。gitdir、commondir、仓库属性及忽略文件的外部引用在正文读取前拒绝，不能向父目录另找仓库；固定系统工具配置按独立宿主允许列表处理。
- **AND** Unix 使用相对句柄打开及目录流，Windows 使用有目录枚举权能的相对 HANDLE 并先检查完整身份、重解析及占位状态；不回退绝对源路径枚举。不能可靠保持的链接、特殊文件、物化或名称语义明确拒绝。旧可信采样签名与原语义保持兼容，新模式不会自动授予权限或开放远程工具。

#### Scenario: Captured Git commands never reopen the live source tree
- **WHEN** 完成私有捕获后、实际 Git status 或内容探测之前，源根、父目录或工作文件名称被替换
- **THEN** 子进程的 cwd、工作树、index、对象库、配置和引用均只指向同一 owner 的实际私有输入；真实命令输出仍来自捕获内容。随后源变化复核须拒绝整体成功，不能以最后一个错误代替已经发生的源读取隔离证明。
- **AND** 验收记录实际中间命令输出及源读取哨兵；用真实 Git 的旧实时模式作正控制，不把缺函数编译错误、发现阶段失败或假输出算作隔离回归。
- **AND** 若原生句柄在替换前禁止祖先目录 rename，验收须确认原范围根与源身份未变、私有命令和终检正常，并在 owner 释放后确认 rename 成功；允许替换的平台仍须验证替换后终检拒绝。不得 ignore 平台案例或关闭 SHARE 等原生保护来强行制造替换。

#### Scenario: Scoped private capture preserves ordinary bounded Git inputs
- **WHEN** 预算内的普通工作树包含隐藏、未跟踪、忽略文件及各层属性，并涉及原 index 的高精度时间、文件模式或行尾语义
- **THEN** 捕获和复核共享原始输入字节、条目、期限与取消额度，私有流式写入独占创建并按实际分配及卷余量准入；不使用链接回源，不为第二轮读取补充预算。必要模式及时间通过实际私有句柄核验，不能以改变 Git 信任配置或刷新源 index 伪造 clean。
- **AND** clean/dirty、unborn、stash、本地跟踪差分、SHA-1/SHA-256、受范围约束的 linked worktree、ignore/attributes、CRLF、filemode 与 racy-index 由真实 Git 差分及对应原生环境验收。私有捕获完成不代替授权持久任务、证据发布、provider 或生产平台门禁。

#### Scenario: Git status output exhaustion is explicit
- **WHEN** 普通宽工作树的 status NUL 记录耗尽整次累计管道额度（兼容默认 1 MiB）
- **THEN** 返回明确资源失败并清理，不返回局部 dirty 数或假 clean。20k/200k 索引与查询基准不能作为 Git sampler 同等工作树规模可成功的证据；提高可配置输出额度仍须保留整次累计期限、取消和字节上限。

#### Scenario: Flat object copies preserve ordinary Git semantics
- **WHEN** 普通 SHA-1/SHA-256 仓库、仅 packed 对象的仓库、linked worktree 或 shallow 仓库在预算内采样
- **THEN** 实际 dirty、stash、HEAD 和本地 upstream 差分保持一致，源对象身份、内容及高精度修改信息保持不变；私有分配和卷余量不足时公共采样返回明确资源失败并清理。全量对象复制和末段复核成本随原始对象字节增加，超限拒绝不得称为严格 RSS 上限或生产提速。

#### Scenario: Git for Windows filesystem cache remains typed data
- **WHEN** 已捕获宿主或仓库配置包含 Git for Windows 的 `core.fscache` 布尔字段，或后续层覆盖该值
- **THEN** 只按明确的布尔类型校验并依原顺序回放，普通 dirty、stash 与 upstream 观察保持实际 Git 语义且源元数据不变；非法值和未知 `core.*` 仍拒绝，不能为接受缓存字段放开 fsmonitor、filter 或外部程序。缓存限定于各次 Git 子进程，启用期间的 stat/目录缓存不构成原子工作树快照或严格 RSS 上限；Windows 实际行为须由原生夹具验收。

#### Scenario: Materialized private metadata remains verifiable
- **WHEN** 工具执行期间私有 index、配置或引用被原地修改，即使身份、长度与分配量未变，或私有根路径被替换
- **THEN** 末段拒绝已登记文件版本变化，只有 owner 自身的受控写入能更新水位；清理不得将替换来的陌生目录当作 owner 删除。此复核是变化检测，不是对同权限进程或整个文件系统的原子隔离。

#### Scenario: Racy index timestamp is copied faithfully
- **WHEN** 工作文件同长度修改后恢复 mtime，且原 index 时间要求 Git 重新读取内容
- **THEN** 私有 index 仍发现该修改；复制字节后保留并通过句柄读回确认原 index 的高精度 mtime。无法表达时明确拒绝或使用另经原生验收的保守重验策略，不能用复制完成时间或零时间假定等价。

#### Scenario: Git sampling leaves source metadata unchanged
- **WHEN** 采样普通、特殊或缺对象的仓库，以及准备失败、超限或取消
- **THEN** 不执行仓库 filter/fsmonitor/远端 helper，不产生联网或原 index/config/refs/logs/对象时间及 lock 文件写入；使用受控哨兵和源数据前后核验验收。元数据复核只是变化检测，不得称为原子仓库快照或严格 RSS 上限。

#### Scenario: Unborn or failed Git observation
- **WHEN** 仓库尚无提交但存在未跟踪或暂存文件，或 HEAD/upstream 查询因预算、取消、I/O 或格式错误失败
- **THEN** 尚无提交时仍报告实际修改；探针失败不得降级成没有提交、没有 upstream 或零计数。

#### Scenario: Native Git status records
- **WHEN** 文件名包含换行、非 UTF-8 或选项前缀，未跟踪目录含多个文件，或状态包含 rename/copy 的两个路径
- **THEN** 按 NUL 分隔的原生状态记录计数，每个未跟踪文件独立计入，rename/copy 的源和目标仅计为一个状态；未终止/非法记录必须返回错误，不能用显示行数或 lossy 文本替代。

#### Scenario: Git reference and numeric failures
- **WHEN** 当前分支引用损坏、工具正常非零退出，或已成功命令返回非法 OID/计数
- **THEN** 仅明确缺少引用可表示 unborn 或未知跟踪引用；其他错误停止整次采样，非法及溢出计数不能被解释为 unknown 或零。

#### Scenario: Dangling symbolic branch
- **WHEN** HEAD 的直接分支已存在，但该 symbolic branch 指向缺失引用
- **THEN** 保留直接分支身份并返回无法解析的错误，不能通过递归解析将其降级成 unborn。

#### Scenario: Complete stash enumeration
- **WHEN** stash reflog 含损坏记录或缺失的留存 commit，或采样期间日志改变
- **THEN** 返回错误，不能把 Git 静默跳过后的局部列表当成完整 stash 数；只支持能验证的引用后端。合法 drop、无 rewrite 删除及 expire 后的空日志保留 Git 语义，旧 OID 不要求连续或仍存在。
