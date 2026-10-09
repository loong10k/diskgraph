## Purpose

定义 DiskGraph 从已有库基础到本地和远程产品的验收证据，覆盖协议、权限、数据安全、真实平台、故障恢复和智能体效率，并明确私有交付与未来公开发布的授权边界。

## ADDED Requirements

### Requirement: RE-01 Behavior driven completion
每个实现增量 SHALL 从对应规格场景建立失败测试、最小实现及回归证据；文件存在、编译、返回 HTTP 200、任务勾选均不能单独证明完成。

#### Scenario: Missing behavior
- **WHEN** 命令存在但输出 stub 或不满足负向场景
- **THEN** 任务保持未完成，不能计入已交付命令数。

### Requirement: RE-02 Security and fault gates
写能力发布 SHALL 通过越权、伪造批准、重放、路径竞态、崩溃、断线、并发冲突、满盘、部分完成和恢复冲突测试；真实用户数据不得作为破坏性测试对象。

#### Scenario: Unverified write adapter
- **WHEN** 某平台写适配器仅编译通过
- **THEN** 该平台保持写能力关闭直至实际门禁通过。

### Requirement: RE-03 Efficiency evaluation
系统 SHALL 对大目录定位、项目归属、历史增长等任务进行同模型同数据的多次对照，分别记录冷索引、热查询、同步、正确率、工具调用、返回/驻留上下文、时间及存储成本，不套用 CodeGraph 数字。

#### Scenario: One query benchmark
- **WHEN** 仅热缓存单次查询更快
- **THEN** 不足以声称整体提效；补齐准确性和累计成本比较。

#### Scenario: Native scan benchmark has an incomplete identity capture
- **WHEN** Linux 配对扫描验收的持久观察数、成功捕获数或缺口数不满足原精确门禁
- **THEN** 仍然失败，并按该快照汇总固定缺口原因计数；未知值映射为固定 invalid_gap，不输出路径、文件名或原始观察。诊断失败不得改成通过，成功路径不额外执行缺口聚合。

#### Scenario: Windows desktop worker qualification with built-in PowerShell
- **WHEN** Windows 台式机使用系统 PowerShell 5.1 从真实 Cargo artifact 准备受信验收副本
- **THEN** 路径准入与SHA256计算不依赖仅新版.NET提供的API；相对路径、盘符相对路径、根相对路径均拒绝，完整绝对路径的独占副本与原字节/摘要一致，已有副本拒绝覆盖且不得重写已发布环境。
- **AND** 真实退出状态C夹具以明确UTF-8输入编译，继续启用/WX；JSON回执及共享环境文件明确使用UTF-8，不依赖系统代码页或PowerShell默认重定向编码。两种原始32位退出码须实际执行验证。

#### Scenario: Full packaged load includes fixture retirement
- **WHEN** 完整打包负载已完成原文件数、路径覆盖、并发读取及超限拒绝检查
- **THEN** 记录真实临时目录清理的开始、结束及耗时，在目录实际删除后才输出成功报告；清理失败保留原异常，不输出清理完成或成功报告。
- **AND** Windows 原完整流程 300 秒期限仍包含准备、产品调用、验证和清理；阶段计时及清理诊断不得缩小负载、放宽门槛或证明产品监督恢复完成。
- **AND** Windows验收夹具清理可用最多4个线程删除本轮实际创建的固定文件集合，随后仍执行原临时目录完整回收；任意删除失败保留错误，不输出清理完成或成功。全部清理时间计入原300秒，无延后、后台遗留或少建文件；本机并行逻辑测试不证明原生性能收益或生产写能力。

### Requirement: RE-04 Platform and protocol proof
发行 SHALL 区分单元/集成测试、真实客户端、目标 OS、移动真机与升级恢复证据；三种传输分别完成兼容性验证，产物附版本/校验和/依赖许可信息。

#### Scenario: SSE format but wrong protocol
- **WHEN** 现代 HTTP 返回事件流但未测试旧 HTTP+SSE 客户端
- **THEN** 不能宣称旧版兼容交付。

#### Scenario: Mobile build only
- **WHEN** 生成 Swift/Kotlin 绑定但未真机调用
- **THEN** 仅声明绑定生成完成，不声明移动功能可用。

### Requirement: RE-05 Private release and explicit stages
项目 SHALL 保持私有并按阶段启用已验收能力；公开仓库、公开包发布、生产部署或破坏性管理必须另获明确授权，规划完成不触发实施。

#### Scenario: Planning validated
- **WHEN** OpenSpec 文档与任务通过校验
- **THEN** 只说明规划完整，所有未执行实现任务仍未勾选，不自动发布或 apply。

### Requirement: RE-06 Desktop read-only production gate
本次生产就绪范围限定为 macOS、Linux、Windows 的只读 CLI 与 MCP。每个目标 OS SHALL 通过可复现构建、测试和实际二进制的 stdio、现代 HTTP 与 legacy SSE 协议验收；网络传输必须使用隔离库、有效签名 token 和真实数据库授权，并覆盖未认证及恶意 Origin 拒绝。发布证据 SHALL 包含目标 OS、版本、制品摘要、升级/回滚演练、资源预算和受控运行观察。未取得某 OS 的实际证据时不得宣称该 OS 生产就绪。

#### Scenario: Legacy acceptance script lacks authentication
- **WHEN** 验收脚本启动 legacy SSE 服务但没有配置认证与隔离授权
- **THEN** 脚本失败，不能以跳过安全门禁的方式标记协议通过。

#### Scenario: CI source check without released binary proof
- **WHEN** 三平台源码测试通过，但对应制品尚未完成真实协议和升级/回滚验收
- **THEN** 只能标记源码门禁通过，不能标记完整生产就绪。

### Requirement: RE-07 Full platform production readiness
全平台目标 SHALL 覆盖已定义的 macOS/Linux/Windows 桌面能力、Swift/Kotlin 原生接口、Android SAF 与 iOS 授权文档。每项平台能力必须同时具备实现、正式库包、真实宿主行为和升级恢复证据；Android/iOS 真机验收、签名及生产部署仍按独立授权与环境条件执行。禁用或 unsupported 是安全边界，不得作为该能力已完成的证明。发布必须依赖同一源码版本的行为门禁。

#### Scenario: Provider is only a capability declaration
- **WHEN** URI 类型和 provider 能力声明存在，但无法在授权生命周期内发布真实快照
- **THEN** Android/iOS provider 保持未完成，不以绑定生成或桌面 CLI 通过替代。

#### Scenario: Native library package lacks host proof
- **WHEN** XCFramework/AAR 只有生成源码、编译或解包证据
- **THEN** 不标记原生嵌入就绪，必须验证库加载、SQLite 共存、查询、取消与授权撤销。

#### Scenario: Windows root validation cost stays diagnostic
- **WHEN** 完整负载显式开启DISKGRAPH_SCAN_DIAGNOSTICS=1且原staging阶段完成
- **THEN** 输出截至该阶段已返回根校验的累计耗时、调用数和保留链句柄数，保留原根链/当前名称绑定校验与所有任务检查；此分项与观测编码时间重叠，不能相加成总耗时。
- **AND** 默认不输出；负载转存只接受固定标签、有限数字与固定字段，不转存路径、正文、未知标签或额外后缀。计数饱和不回绕，原返回值、错误及panic不被诊断替换。
- **AND** 纯计数器和parser本机测试不证明Windows原生根校验或正式200k/300秒验收；真实平台分项数据仍须绑定当前源码和实际二进制。

#### Scenario: Sustained authenticated HTTP read qualification
- **WHEN** 对实际CLI创建的隔离快照显式指定持续请求时长
- **THEN** 在同一个实际MCP服务进程中重复执行已授权查询，逐次核验真实scope及非空结果；请求沿用固定总截止时间并限制单次等待，任意HTTP/业务失败或错误归属均使验收失败。
- **AND** 只保留有限延迟样本，记录实际请求数、实际时长和样本窗口；每次响应读取有明确字节上限。时长结束后仍须执行原实时撤权拒绝与原服务退出检查。
- **AND** 该持续读取不自动证明并发、SSE连接回收、RSS无增长或长期生产稳定性；模拟测试及未执行的原生运行均不得标记长期运行门禁完成。
- **AND** 三桌面包验收显式执行60秒持续读取烟测并保存完整HTTP验收报告；烟测不能替代长期运行。原HTTP脚本默认短验收、外层300秒期限及20k/200k负载各自原门禁不变。
- **AND** HTTP报告记录实际CLI与MCP镜像的前后内容摘要及字节数；镜像变化或超过有限读取预算时拒绝完成证明。该内容核对不替代操作系统执行镜像绑定或源码SHA关联。

#### Scenario: Load storage observations distinguish database and WAL growth
- **WHEN** 隔离20k/200k负载在范围注册后、扫描发布后和并发查询后记录存储成本
- **THEN** 只计两库及各自WAL/SHM的实际文件长度，分别报告三个时点及扫描/查询的有符号增量；checkpoint导致WAL缩减时保留负值，不把缺失WAL或文件总长度当作累计写入字节。
- **AND** 原database_and_wal_bytes字段兼容保留，原时间门禁、四客户端并发、精确节点/路径覆盖、拒绝部分发布和回收验证不变。该成本不是卷分配量、峰值WAL或RSS；未执行目标平台程序时不宣称原生负载通过。

#### Scenario: Last HTTP dispatch interval has insufficient budget
- **WHEN** 40次/秒持续读取的原总期限剩余不足max(25ms发起间隔, 本轮已观测最大请求耗时)，包括token准备消耗后的剩余额度
- **THEN** 不再准入新的网络请求；继续等待原观测截止时间，报告实际准入保留值，不缩短60秒观测、不刷新期限、不补发积压。已发出的请求仍使用原剩余时间且任何超时或HTTP/业务失败均失败，不能将此检查用作吞掉末次错误的重试机制。
