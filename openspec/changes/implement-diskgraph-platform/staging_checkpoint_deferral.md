# 暂存批次的 checkpoint 与控制 fence 分离

问题：节点暂存和 Linux Unix 观测旁表的真实提交处于控制库 `with_job_fence` 内；SQLite 默认自动 checkpoint 同步发生在图库 commit 中，因而磁盘维护也占用控制库互斥量，阻碍撤权、续租与 SSE 实时授权。正式发布已分离维护，但暂存批次尚未分离。

本增量保留 WAL/NORMAL、原自动 checkpoint 页阈值、批次事务、原授权/取消/租约检查与容量门禁。新增可信内部延迟暂存入口，在真实 commit 的 WAL 回调中只捕获实际页数，不执行文件维护、不缓存授权；成功、错误、panic 均移除回调并恢复原连接阈值。只有原阈值大于零且实际提交页数达到阈值才请求维护，不能按文件尺寸估算页数。旧可信暂存 API 保留原行为。

Engine 在同一原 fence 中提交节点及适用的 Unix 旁表，保留是否需要维护；先显式释放控制库 guard，再执行原图库连接上的 PASSIVE checkpoint，然后进入下一批次。不得将全部暂存 checkpoint 推迟到最后发布，亦不得因此忽略 WAL 容量。fence 终态错误优先于维护错误，已实际提交的批次即便控制事务末检失败也需处理其维护请求。PASSIVE 被读事务阻挡不视为清理完成，既有容量门禁仍计算实际文件占用。

验收先在真实磁盘 SQLite 验证行为失败：阈值为 1 时延迟提交可由原连接读取，但不带 WAL 的主库副本尚无该批次；锁外显式维护后主库副本含全部批次。覆盖关闭/未达到阈值、回滚、panic、连续批次及 Unix 旁表；配置恢复后普通写入仍可正常触发默认维护。Engine 需保留原回归，并在实际平台验证不改变授权、节点覆盖和同代恢复语义。本项单独通过不能关闭 Windows 200k/300 秒、有限退出或全平台生产门禁。

实现：节点和 Unix 旁表各提供兼容入口之外的可信延迟入口；原连接的临时 WAL 回调仅写入稳定 Box 内的 Cell。Connection 借用覆盖注册期，回调拆除先于 Box 释放，正常恢复失败明确返回；错误/panic 的 Drop 也先拆除回调再尝试恢复。维护责任先累计到调用者变量再恢复配置，所以恢复 SQL 失败不能丢掉已实际提交的页数。Engine 的 `staging_wal::finish` 消费原控制 guard，显式释放后才观察测试边界和处理 PASSIVE；原图库写 guard 保留，fence 结果优先于维护结果。原节点/旁表事务、授权及容量入口不变。同步 SQLite/OS 调用仍为协作边界，不声明硬墙钟上限。

真实 TDD：新接口最初仅委托原暂存逻辑，磁盘 SQLite 主库副本实证 2 个节点而期望 0，0/1 RED；该失败先于维护标志断言，证明的是旧自动 checkpoint，而不是占位返回值。实现后 1/0，扩大 6/0，覆盖原阈值关闭/未达到、旧 API 自动维护恢复、实际写入后取消/panic 回滚、配置恢复 SQL 被拒绝时的原维护责任、Unix 观测旁表。macOS 存储全目标单元 370/0/8，其余候选10、认领13、迁移2、结构3均通过；性能探针4 ignored，不计覆盖。Store/Engine 严格全目标 Clippy 通过。

Windows 第一轮实际结果：新增 6 个暂存测试与结构3通过；真实 Engine 观测5/0，包含维护入口前同一 Engine 控制锁可取得、真实撤权后不发布。存储默认并发整包为 380/2/8，失败是两项原控制库写入时钟探针，原 BEGIN 实际约3秒后返回；不能写成整包成功。源码、原 argv、原 worker/driver、日志摘要和失败阶段数值见 `windows_staging_checkpoint_2026_10_09.json`。随后两案独立诊断2/0，分别实际约150/40 ms，SQLite3.53.2 未启用 ENABLE_SETLK_TIMEOUT，否决编译开关假设；不能以此擦除默认整包失败，也尚未确定调度、VFS或原生I/O原因。诊断见 `windows_control_boundary_staging_diagnostic_2026_10_09.json`。

结构复核曾因 scan_execution 增长到510行实际失败5/1；将消费控制 guard 的维护边界按职责提到独立 staging_wal 模块后，原文件499行，原结构门禁6/0，不修改500行上限。该最后源码调整的 Windows 定向验证仍待结果。首次 Windows 测试驱动字符串替换错误导致 Python 语法错误，未进入编译/测试；修正并先验证驱动语法后才取得上述原生结果，不计作产品 RED。

最后源码首轮 Windows：存储6/0、结构6/0及3/0；Engine4/1，新增撤权断言实际收到 StaleOwner。核实原发布 fence 将撤权/cancel_requested 归为 StaleOwner，原错误保存合同也禁止把真实 owner 错误统一成 PermissionDenied。修正新增测试接受这两个明确错误，并额外检查实际 revoked、cancel_requested、原 owner/fence、Cancelled/Failed 终态，以及 staging/snapshots/revisions/collector_runs 全部为零；没有放宽生产错误处理或发布门禁。随后同一原生 worker、当前源码与默认测试并发验证：6/0、5/0、6/0、3/0，见 windows_staging_revocation_settlement_2026_10_09.json。严格全目标 Clippy 再次通过。

状态：实现与本机/Windows 子能力已验证；同 SHA CI、默认并发写期限失败及 Windows 200k/300 秒仍开放，不关闭父项。不承诺此次改动已降低整次扫描时间、RSS或端到端 p95，性能需要实际 release 对照。
