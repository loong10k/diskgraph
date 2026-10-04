## Purpose

提供可发现、可脚本化且与 MCP 和原生接口语义一致的丰富命令体系，明确查询、索引管理及文件修改的边界，避免把命令数量等同于外部 Shell 依赖或已完成的功能。

## ADDED Requirements

### Requirement: CMD-01 Complete business catalog
系统 SHALL 交付 scope、index、sync、status、snapshots、changes、growth、explore、search、node、children、top、related、explain、impact、candidates、duplicates、read、move、copy、trash、restore、purge、plan、apply、operations、serve、install、doctor 共 29 个命令或命令族，按阶段及权限声明可用性。

#### Scenario: Help and capabilities
- **WHEN** 用户查看帮助与能力报告
- **THEN** 所有已交付入口有参数、效果、限制和阶段说明；未实现能力不得显示为可执行成功。

### Requirement: CMD-02 Shared parameters and errors
命令 SHALL 支持适用的显式 scope、revision、分页/预算与结构化输出，定义稳定退出码和可区分错误；本机和远程资源不得因当前工作目录不同而误解析。

#### Scenario: Scripted denied query
- **WHEN** 脚本执行无权 scope 的 JSON 查询
- **THEN** 获得非成功退出码和结构化 permission_denied，不混入进度文本。

#### Scenario: C03 explicitly selects a persistent Git collector
- **WHEN** 调用 `sync --scope S --revision R --node-id N --collector git`，或 MCP `diskgraph_sync` 携带同名 scope、revision、node_id、collector 字段
- **THEN** Git 模式要求显式 scope、revision 及正整数 node_id，以固定目录节点创建持久 Git 证据任务并返回 job_id；CLI 可用 `--wait` 等待实际终态。无 ContentRead 时 CLI 返回退出码 3 与结构化 permission_denied，MCP 返回同一业务错误，不能将合法 Git 模式误报为未知参数。
- **AND** 省略 collector 的旧 sync 保持原扫描和等待语义，不要求 Git 的正文授权；合法 Git 请求与旧 sync 必须分别通过真实进程/公开服务入口验收，不以 schema-only 检查替代执行、发布及状态查询闭环。

#### Scenario: HTML report contains untrusted names
- **WHEN** 索引名称或根路径含 HTML 标签、事件属性或 `</script>` 序列
- **THEN** 离线报告只把它们当数据呈现，不新增可执行元素或脚本片段。

### Requirement: CMD-03 Mutation commands create plans
move/copy/trash/restore/purge SHALL 默认只创建或预览不可变计划；apply 才进入批准校验和执行。read、duplicates 和 index 的非修改文件行为仍需各自的内容/管理权限。

#### Scenario: Trash without apply
- **WHEN** 用户执行 trash 计划命令
- **THEN** 返回影响、恢复条件和计划 ID，不移动文件。

### Requirement: CMD-04 Complete MCP mapping
业务查询、授权后的索引管理与操作能力 SHALL 有对应 MCP 服务方法；默认工具配置可精简，但已获授权的精确查询不得仅能通过模糊 explore 间接使用。serve/install 等宿主启动配置保留在 CLI，不通过远程工具修改调用者宿主。

#### Scenario: Detailed query profile
- **WHEN** 管理员启用完整只读工具组
- **THEN** 客户端可发现并调用 search/related/impact 等工具且共享查询语义。

#### Scenario: Strict client constructs tool arguments
- **WHEN** 客户端按 tools/list 的 inputSchema 构造搜索、历史、目录、关系及任务状态请求
- **THEN** schema 声明处理函数实际支持的参数、类型与必填项；未知字段和错误类型被明确拒绝，不能静默使用默认值。

#### Scenario: C03 Git discovery matches the restricted request contract
- **WHEN** 管理工具组公告 diskgraph_sync，客户端选择 collector=git
- **THEN** inputSchema 声明 collector 的 git 允许值、revision 字符串及 node_id 正整数类型，实际校验要求显式目标；未知 collector、缺失目标、错误类型与未声明字段均明确拒绝。公告及实际处理均不接受客户端 path、argv、authority 或 network 字段，也不把这些值注入可信执行上下文。
- **AND** 真实已认证请求的能力上限与数据库 grants 求交，返回的任务持久绑定已验证主体；接口测试保留无 collector 的旧 sync 正控及合法目标的业务拒绝负控，避免把参数拒绝误记为授权检查成功。

#### Scenario: Explicit historical node query
- **WHEN** node/top/children/explore/search/candidates 携带已授权旧 revision，或 node 携带非根 node_id
- **THEN** 查询和响应身份绑定指定 revision 与节点；scope 不匹配被拒绝，不改查 latest 或根节点。

### Requirement: CMD-05 Scope and aliases are not deletion
scope remove SHALL 只撤销注册/访问，索引删除必须单独显式请求；children 的 ls 等别名不计为新业务能力，不要求存在同名系统命令。

#### Scenario: Remove a scope
- **WHEN** 用户移除 scope 且未批准文件操作
- **THEN** 被扫描文件和恢复记录不被删除。
