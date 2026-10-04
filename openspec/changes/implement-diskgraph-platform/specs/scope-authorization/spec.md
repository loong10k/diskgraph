## Purpose

定义本机与远程访问资源的身份和授权边界，使目录元数据、文件内容、索引管理及文件修改各自遵循最小权限，并防止跨用户、跨服务器或路径变化导致的数据越权。

## ADDED Requirements

### Requirement: SC-01 Explicit scope registration
系统 SHALL 仅扫描和操作管理员或本机可信用户显式注册的根目录范围；普通查询和清单引用不得自动注册、扩大范围或授予权限。

#### Scenario: Manifest outside the scope
- **WHEN** 项目清单引用授权范围外的输出目录
- **THEN** 保留未解析引用及原因，不扫描或读取该目录。

#### Scenario: Remote root injection
- **WHEN** 远程查询携带未注册的根路径
- **THEN** 返回范围拒绝且不创建 scope。

### Requirement: SC-02 Server and principal bound resources
系统 SHALL 将资源引用绑定 server、scope、revision 与对象 ID，并对每次请求按主体校验范围；知道或猜到 ID 不构成授权。

#### Scenario: Foreign server identifier
- **WHEN** 客户端提交另一服务器的对象 ID
- **THEN** 拒绝该引用，不将其按本地路径解析。

#### Scenario: Denied aggregate
- **WHEN** 无权主体请求其他用户的 top、搜索或统计
- **THEN** 不返回对方名称、数量、尺寸、证据或内容。

### Requirement: SC-03 Separate capabilities
系统 SHALL 默认仅允许已授予的元数据查询，并分别授权 content:read、index:manage、files:move、files:copy、files:trash、files:restore、files:purge 与 admin:scope；FFI、CLI 与所有传输执行同一策略。

#### Scenario: Read only cannot mutate
- **WHEN** 只读凭据直接调用隐藏的 apply 或 FFI 写入口
- **THEN** 服务拒绝执行，不以工具未列出代替权限检查。

#### Scenario: Metadata is not content consent
- **WHEN** 主体只有目录查询权却调用 read
- **THEN** 返回权限不足，不读取文件正文。

### Requirement: SC-04 Revocation and resource boundaries
系统 SHALL 在执行前重新校验撤权、保护规则、挂载身份、源与目标范围，并阻止符号链接、路径穿越或目录替换逃逸；无法满足安全前提的平台不得降级成不受控操作。

#### Scenario: Root changed
- **WHEN** 计划批准后根挂载或中间目录被替换
- **THEN** 计划失效，拒绝继续修改。

#### Scenario: Destination outside grant
- **WHEN** 移动目标不在主体授权范围
- **THEN** 拒绝计划，不因源目录合法而接受目标。

### Requirement: SC-05 Remote authentication and privacy
远程服务 SHALL 对网络请求实施身份认证、逐请求授权、加密传输和 Origin 校验策略，默认不公开监听；索引留在服务器，返回内容遵守导出策略且默认不上传凭据或完整文档。

#### Scenario: Anonymous network call
- **WHEN** 未认证客户端请求 HTTP 查询或旧版 SSE 工具
- **THEN** 拒绝且不泄露索引数据。

#### Scenario: Model exposure
- **WHEN** 云模型宿主仅被授予去标识摘要导出
- **THEN** 响应移除未授权原始路径、内容和秘密，日志同样脱敏。

### Requirement: SC-06 Request-bound identities and revisions
服务 SHALL 对每次远程请求执行 token 能力与实时主体授权的交集，并验证 revision 的持久 server/scope 归属。

远程 Index、Sync 及实时证据任务 SHALL 保存服务端从已认证请求取得的原主体、签发方、能力上限与绝对到期约束，不持久保存 bearer。持久任务与断线恢复不构成超越原请求能力或到期时间的无限委派；runner 的可信本地身份不得补足、替换或续期该约束。入队、实际执行和发布按任务所需权限与当前数据库授权取交集，租约续租和重领不得延长认证有效期。旧可信本地兼容入口保持独立，不进入远程执行旁路。

#### Scenario: Token without capabilities
- **WHEN** 有效 token 无权限 scope 请求目录或工具
- **THEN** 不返回索引信息，不使用本地管理员身份。

#### Scenario: Foreign revision
- **WHEN** 仅获 scope A 权限的主体携带 scope B revision
- **THEN** 返回拒绝，不泄露 B 的数据。

#### Scenario: Impact traversal with a foreign revision
- **WHEN** 仅获 scope A 权限的主体对 scope B revision 调用关系影响分析
- **THEN** 在读取任何关系前按 B 的实际归属拒绝，不以客户端传入的 A scope 代替 B。

#### Scenario: Revoked queued scan
- **WHEN** 任务排队后 scope 被撤销
- **THEN** 不扫描或发布，并停止已有运行任务。

#### Scenario: Authenticated queued index and sync expire before execution
- **WHEN** 已认证且具备实时 IndexWrite 授权的远程主体经 Index 或 Sync 获得持久 queued job，而原 token 在 runner 执行前到期
- **THEN** 原任务不扫描、不发布 revision，并在有界调度内保存拒绝、失败或取消的可查询终态；不能因只保留主体或查询到当前数据库 grant 而继续执行。
- **AND** 续期后同主体可按当前任务查看权限查询原 job；此查询及连接重建不改写原任务能力或到期时间，不创建替代任务。

#### Scenario: Running request authority expires before publication
- **WHEN** 远程任务已进入实际执行，随后原认证到期或所需实时 grant 被撤销
- **THEN** 同一代次观察取消或拒绝，发布 fence 必须重新检查原请求上限、到期、当前 scope/grants、owner、lease 与取消；已完成的前段采样不能使过期或撤权发布成功。
- **AND** 执行预算从成功认领后开始，队列等待不消耗运行预算；原绝对认证到期仍在排队及重领期间流逝，续租不得续期认证。

#### Scenario: A narrower request cannot inherit a stronger active job
- **WHEN** 同主体以不同能力上限或不同到期上下文请求相同范围的任务，或客户端在参数中伪造主体、签发方、可信本地模式或能力
- **THEN** 仅服务端已认证上下文决定身份约束；欠缺所需能力先拒绝，其他上下文不得通过合并继承较强旧任务或延长旧任务的认证有效期。

#### Scenario: Legacy jobs have no attributable request authority
- **WHEN** 旧控制库的 queued 或 expired-running 任务没有可确认的请求来源与能力上限
- **THEN** 远程自动 runner 不将缺失记录当成可信本地授权，不进行扫描或发布；保留原记录的授权查询和取消，并明确要求可信本地恢复或重新授权后重新入队。
- **AND** running 任务的存活租约不得直接抢占；只有租约过期后才能按条件终结、隔离或明确恢复。旧可信本地兼容执行与远程严格执行路径分离，带有请求约束的任务不能借兼容入口绕过该约束。

#### Scenario: Git jobs cannot lose content authority through compatibility APIs
- **WHEN** Git 证据任务通过原可信认领、续租或发布 fence 入口执行
- **THEN** 持久任务类型强制 MetadataRead、IndexWrite、ContentRead 与原 token 上限及实时 grants 的交集；调用方省略权限集合不得降低三项要求，缺少合法固定目标输入不得退回扫描或实时工作树采样。
- **AND** 固定目标包含实际本机 server、scope、base revision、正整数 node 与服务端有限配置，不含 bearer、任意路径或命令；重开时重新验证类型、归属、原始长度、摘要和输入版本，同主体不同目标或不同原请求约束不得合并。
