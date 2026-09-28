## Purpose

让用户能够独立安装 DiskGraph 并连接不同智能体，同时复用已有目录索引、保留其他工具配置及用户隐私，并以真实工具调用和任务结果验证集成而非只验证配置文件存在。

## ADDED Requirements

### Requirement: AI-01 Standalone local and remote usage
发行物 SHALL 允许用户不安装 Rust 开发工具链、disktree 应用或 PruneX 即可运行支持平台的预编译服务；本地与远程使用都需明确 scope 和能力。

#### Scenario: Fresh host
- **WHEN** 用户在受支持主机安装发行物并注册目录
- **THEN** 可建立索引、退出重启后查询，不依赖 PruneX 数据库。

### Requirement: AI-02 Reversible targeted configuration
install SHALL 提供配置预览和按客户端注册/移除，保留其他 MCP、注释和无关权限；重复安装幂等，配置使用明确程序/数据位置而非依赖宿主 cwd。

#### Scenario: Existing client configuration
- **WHEN** 客户端已有其他 MCP 和注释
- **THEN** 安装后仅目标配置变化，再运行不产生重复项，移除保留其他内容。

### Requirement: AI-03 Index reuse and no surprise scans
不同智能体 SHALL 在相同权限域复用已发布索引，不为每个 stdio 进程重复全量扫描；工具说明须区分未索引、过期、部分覆盖和拒绝访问。

#### Scenario: Two agents query
- **WHEN** 两个宿主同时查询同一个 scope
- **THEN** 复用同一已发布 revision；需要同步时合并作业而不是重复扫描。

### Requirement: AI-04 Real client acceptance
独立智能体交付 SHALL 在至少两个不同宿主上验证工具发现、调用、重连、索引复用及真实调查任务，不能以配置写入或协议连接成功替代。

#### Scenario: Handshake only
- **WHEN** 测试只完成连接没有工具调用
- **THEN** 集成状态仍为未验收，不标记宿主支持完成。
