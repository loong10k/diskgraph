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

#### Scenario: Git configuration execution
- **WHEN** 仓库配置提供 fsmonitor、clean/process filter 或其他外部程序能力
- **THEN** 只读 Git 采样必须使用不执行这些程序的受限配置或明确拒绝；配置检查后的竞态不得恢复外部执行，不能仅按命令名称声称离线。

#### Scenario: Unborn or failed Git observation
- **WHEN** 仓库尚无提交但存在未跟踪或暂存文件，或 HEAD/upstream 查询因预算、取消、I/O 或格式错误失败
- **THEN** 尚无提交时仍报告实际修改；探针失败不得降级成没有提交、没有 upstream 或零计数。
