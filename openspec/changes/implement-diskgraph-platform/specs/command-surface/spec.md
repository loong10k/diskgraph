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

### Requirement: CMD-05 Scope and aliases are not deletion
scope remove SHALL 只撤销注册/访问，索引删除必须单独显式请求；children 的 ls 等别名不计为新业务能力，不要求存在同名系统命令。

#### Scenario: Remove a scope
- **WHEN** 用户移除 scope 且未批准文件操作
- **THEN** 被扫描文件和恢复记录不被删除。
