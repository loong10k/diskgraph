## Purpose

使 PruneX 和其他原生宿主复用同一 Rust 领域、存储及受控执行能力，同时尊重桌面与移动端权限差异、URI 资源语义和 SQLite 链接约束，避免把编译成功等同于真实平台能力。

## ADDED Requirements

### Requirement: PF-01 Shared native API
Rust/Swift/Kotlin 入口 SHALL 使用同一版本化核心服务与授权语义，支持后台作业、分页、取消及结构化错误，不要求 PruneX 经 MCP 子进程调用本机引擎。

#### Scenario: UI cancellation
- **WHEN** Swift 或 Kotlin 宿主取消长扫描
- **THEN** 界面线程不阻塞，作业按契约停止且不错误推进 latest。

### Requirement: PF-02 Separate storage ownership
图索引及服务端操作记录 SHALL 由 Rust 管理，PruneX 自身会话和界面状态由其业务存储管理；同一进程原生数据库依赖需验证链接及生命周期，不因使用两个文件就假定无冲突。

#### Scenario: PruneX missing
- **WHEN** 服务端没有 PruneX 或其数据库
- **THEN** 仍可验证授权并查询操作/恢复记录。

### Requirement: PF-03 Platform capability matrix
macOS/Linux/Windows 的路径、身份、占用、回收和操作能力 SHALL 分别实测声明；操作不支持的平台返回 unsupported，不采用危险通用 Shell 降级。

#### Scenario: Windows identity unavailable
- **WHEN** Windows 扫描无法提供可靠卷身份
- **THEN** 历史比较与修改按限制拒绝，不假装跨平台完整支持。

### Requirement: PF-04 Android provider semantics
Android SHALL 接受宿主授权的 SAF/适用媒体观察及 URI，保留 provider 能力、未知大小与授权生命周期；不得把 URI 转成猜测的原生路径或承诺其他应用私有目录清理。

#### Scenario: Provider revokes access
- **WHEN** 扫描或计划后 URI 授权撤销
- **THEN** 后续访问失败并保留 partial/denied，不扩大到文件系统路径。

### Requirement: PF-05 iOS restricted scope
iOS SHALL 从最初契约就限制在 App 自有及用户授予的文档范围，宿主管理安全作用域生命周期、文档协调与恢复能力；不承诺全盘扫描或跨 App 卸载。

#### Scenario: Selected document lifecycle
- **WHEN** 用户选择文档后访问结束或授权失效
- **THEN** 宿主释放访问资源，后续作业重验或拒绝，不保留无限访问假设。
