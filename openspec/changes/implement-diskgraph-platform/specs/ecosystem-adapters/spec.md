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

#### Scenario: Argument injection
- **WHEN** 文件名含选项前缀或 shell 元字符
- **THEN** 作为校验后的独立数据参数处理或拒绝，不成为命令语句或额外选项。

#### Scenario: Shared sample execution budget
- **WHEN** 一次证据采样执行多个子命令或 stdout/stderr 持续输出
- **THEN** 全部命令与两条管道共用绝对期限、累计字节预算与取消状态；失败停止后续工作，不返回完整成功样本。

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
