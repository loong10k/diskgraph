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

#### Scenario: Registration commits scope and grants together
- **WHEN** 请求能力初检成功，scope 注册等待实际控制库写锁期间管理员授权被撤销或策略换代
- **THEN** 在同一 IMMEDIATE 控制事务内重新检查实时 ScopeAdmin，并固定授予权限使用的策略版本；拒绝时不提交 scope 或任一 grant。
- **AND** 新 scope 和三项默认 grant 作为一个控制事务提交。图库负向隔离先完成，再提交控制事务；图库失败、grant 写入失败、期限耗尽或 COMMIT 失败均回滚控制注册，不返回成功。
- **AND** COMMIT 成功后连接清理失败明确返回已提交 scope 的 `RegistrationCommitted` 与原始原因，不把已经提交的注册误报为认证失败或未提交。
- **AND** 跨库不承诺原子提交；图库已持久隔离而控制提交失败时保留保守拒绝及原始审计归属，重试不能解除隔离。认证到期不因写锁等待、图隔离或提交重试而延期。

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

#### Scenario: Registration invalidates an in-flight historical read
- **WHEN** 请求已解析并获准读取历史 revision，随后实际 scope 注册因无损根证明不足而持久隔离该 revision
- **THEN** 读取及响应编码完成后的终检重新查询过滤后的实际归属，拒绝完整和截断数据；scope 权限仍有效不能替代 revision 的当前可访问性。
- **AND** 终检不复用消费者可能持有旧 SQLite 读事务的连接，不获取共享图库写锁；展示已存在的50ms终检窗口只用于授权，不续期数据查询、不重建原始读取账本。
- **AND** 末段能力回调或编码期间完成隔离，同样须在回调及编码之后拒绝；原始归属审计行保留。
- **AND** 双侧历史每轮末段归属观察在能力回调之后开始50ms窗口，两侧共享；最多三轮，不刷新原查询期限，迟到结果仍为超时或明确partial。授权观察失败仍拒绝交付，此窗口不是含同步回调的硬返回上限。
- **AND** 旧 `explain_entity`、`related`、`related_page` 签名同样执行实时末段权限及归属复核；默认一秒查询期限，完整结果字段和分页语义保留。旧签名标记弃用，新对外调用使用能够传递整次字节/节点/期限预算的关系入口。

#### Scenario: Durable withdrawal during terminal authorization
- **WHEN** 真实请求首次获准后，同进程可信控制库入口在末段授权回调中成功持久撤销该请求依赖的 grant 或 scope，且准确绑定实际已打开控制库的请求级负向见证确认此事实
- **THEN** 当前请求返回 PermissionDenied，不交付完整或部分数据；原授权期限、控制库 FULL 和已有精确拒权断言保持，不因撤权调用超过期限把已知拒权改成 BudgetExceeded。
- **AND** 见证只记录成功提交的精确负向事实，不缓存 Allow；注册覆盖初检、消费与末检，同一请求的已知撤权不因随后重授而清除，新请求仍按实时授权判断。

#### Scenario: Unknown or unrelated withdrawal does not manufacture denial
- **WHEN** 原期限耗尽但没有可靠的适用负向见证，或撤销只涉及另一数据库、主体、scope、permission，或事务回滚、提交结果未知、DELETE 未删除授权行
- **THEN** 保留原实时 SQL 与期限错误语义，不把 BudgetExceeded、busy 或 interrupted 批量映射为 PermissionDenied，不延长期限或以路径、server ID、复制库中的 UUID 猜测同一实际数据库。
- **AND** 连接替换或未知原生身份不能借旧订阅误绑定；订阅有界、只保留弱引用，registry 锁不跨 SQL、授权回调、消费者或 OS 调用。

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

#### Scenario: Atomic live scope and grant observation
- **WHEN** 实时权限查询准备策略读取前，独立控制连接已提交实际 scope 的撤销
- **THEN** 同次查询在同一 SQLite 观察中读取 scope、策略及精确 grant，返回拒绝；不得拼接旧 scope 状态与新策略读取后返回允许。
- **AND** 未注册 scope 仍报不存在，未发布策略的可信兼容入口仍返回未定义权限，损坏必需字段仍拒绝；每次调用重新观察持久状态，不缓存授权结果。

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

#### Scenario: Terminal reader actual lock contention
- **WHEN** 末段能力回调确认另一线程已持有真实控制锁，原取锁循环实际观察到 WouldBlock，持锁线程随观察信号立即释放
- **THEN** 正常请求仍须通过实时授权和归属复检并成功返回，不因第一次竞争立即拒绝；成功断言不接受超时作为替代。
- **AND** 持锁线程在观察信号之后持续占锁超过原50ms窗口时，原请求返回 BudgetExceeded；生产能力、锁和SQL观察期限均不增加、不续期。
- **AND** 测试用真实竞争信号确定释放次序，不把 sleep 请求20ms当作实际占锁低于50ms的证明；记录真实耗时，原生CI仍独立验收。
