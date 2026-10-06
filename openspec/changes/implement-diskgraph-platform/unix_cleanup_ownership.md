# Unix 清理失败的所有权与容量语义

本项属于 15.13 原生回收门禁，不完成该父项。

## 验收场景

真实子进程退出、管道 EOF 后，被宿主外部 waitpid 消费原 leader。原 owner 检测到 ECHILD 后必须拒绝基于旧 PID/PGID 的清理，后续重试也不得报告成功。转入恢复 registry 后，每次 drain 必须保留原失败及已占用槽位，不允许新的 reservation 复用该容量。

## 当前证据

既有真实外部回收测试加强后 RED：第二次 cleanup 错误返回成功。移除丢失等待权分支的 cleaned=true 后 GREEN；三次直接重试与三次 registry drain 均保留失败及容量。macOS 原生子进程回归 40 通过、0 失败，唯一默认忽略的隔离辅助夹具由真实测试显式执行。原始日志和源码摘要见 docs/benchmarks/unix_lost_wait_ownership_2026_10_06。

本项不证明 Linux 原生执行、首次检查之后的 wait 竞态、有限时间回收或全平台生产就绪。

## 后续末段 wait 竞态

原先的完成分支还会在末段 wait 报错后置 cleaned=true。既有回归增加初检之前和末段 wait 之前两个真实外部回收时序。末段使用仅测试编译的线程局部单次 checkpoint 调用真实 waitpid，不模拟 ECHILD。原代码实际 RED；修复后 wait 的 ECHILD 撤销旧数值身份权限，保留原错误及容量，其他 wait 错误保留 owner 重试。macOS 40/0/1、结构6/0通过，日志见 docs/benchmarks/unix_cleanup_late_wait_2026_10_06。进程组终止失败路径、Linux 和有限时间回收仍未完成。

## 组终止失败的身份锚点

组终止报错时不得继续 wait 并消费原 leader。即使 leader 已退出，它的未消费等待权仍须保留到组终止确认；registry 保留原 owner 和容量。测试使用真实已退出 child，线程局部持续注入 EPERM，原代码在 waitid WNOWAIT 身份断言实际 RED。修复后连续三次 drain 保留同一 leader 与槽位，解除故障后实际回收并允许再次 reservation。该故障注入验证错误状态机，不代替真实内核权限拒绝验收。macOS 原生41/0/1、结构6/0通过。

macOS 冻结候选599源只覆盖 unix_child.rs、unix_child_group.rs 和 unix_normal_exit_tests.rs，其余字节不变。门禁必须实际执行41项并包含外部等待竞态、组失败保留及原非正PID隔离证明；旧40项不能通过。挂载13/0，本项等待原生CI，不完成15.13父项。日志见 docs/benchmarks/unix_group_failure_retention_2026_10_06。

## macOS 单次期限内恢复合同

ScanWorkerRecovery::drain_until 接收宿主绝对期限；到期或锁竞争返回 false，不取 owner、不发送信号、不消费 wait。锁外单次处置 fresh 私有 session：实际原组终止后，只有完整原组退出观察和原 leader 非阻塞 wait 均完成才能返还容量。活动、到期、原生权限/等待错误和 panic 都将原 owner 放回原槽；外部 ECHILD 不允许重试旧数值组。不得 sleep/retry-loop 或调用阻塞 waitpid 作为这条路径的实现。OS 单调用和归还责任所需短状态锁没有硬墙钟保证。原 drain 兼容接口仍在，有限前端退出不据此宣称完成。

期限入口暂接既有legacy drain时真实过期请求RED为0/1；修复后目标通过。原组失败/末段外部wait/连续槽容量保留、原生与标准WNOHANG缓存、panic和锁竞争回槽均通过。最新并行native_child44/0/1、真实父驱动7/0、结构6/0；原并行正常组查询不完整失败也保留，不因精确和串行复查通过而删除。原日志及源码摘要见 docs/benchmarks/macos_deadline_recovery_2026_10_06。Linux期限恢复、前端有限退出、默认安装扫描和全平台严格检查仍未完成。

## Linux 当前提交验收来源

Linux Engine 原生门禁必须测试 CI checkout 对应的完整当前提交，不得先把旧冻结宿主补丁装配进当前源码。构建前核验所有受版本控制的构建输入与 HEAD blob 一致，拒绝已修改、暂存、未跟踪输入和符号链接；保存提交、逐文件 SHA-256 和清单摘要。构建后再次核验输入未变化。历史装配工具只用于显式历史候选，不能替代当前提交生产验收。原 namespace 隔离、普通 UID、实际 worker 摘要与原 init wait 门禁保持。

## Linux 单次期限恢复验收

Linux 的公开 Recovery::drain_until 必须沿用出生时原 pidfd，不使用数值 PID 重新打开或发信号。真实活动 child 在期限已过时，连续调用不得关闭控制输入、发送信号或消费 wait；原槽不可重用。真实线程 seccomp 拒绝 waitid 时，连续恢复必须传播原 EACCES 并保留同一 owner/容量，未过滤宿主随后可用原 pidfd 实际消费等待。正常清理只有实际 P_PIDFD wait 消费成功且原整个线程组退出后才返还槽位，测试以保留的原 pidfd 重复 wait 得到 ECHILD 和 POLLIN 为证。每轮仅使用 WNOHANG/零超时 poll，不进行内部 EINTR 循环、sleep 或阻塞 wait。前端有限退出仍须独立完成，不能由该接口推出。

三个 Linux runner 的 RED（37438383770、e839fdb）均仅因缺失 drain_until 接口出现 E0599；这是原生编译的接口缺失证据，不称为运行时行为 RED。现接入原 pidfd、零超时轮询和单次 WNOHANG，新增3项用例必须与原5项一起实际通过且保留原wait消费标记。macOS共享registry回归3/0、结构6/0、fmt通过；Linux运行结果仍待CI，有限前端退出和生产父项不勾选。

Linux 期限恢复已通过当前完整源码原生门禁：37438978565 / c98dc1b，x86 stable、x86 Rust1.97、arm64 stable 各8/0/0（原5项+新增3项），逐项包含期限容量保持、真实seccomp wait拒绝、原pidfd实际消费标记和原namespace init wait闭环。原始source清单/worker摘要/日志见 docs/benchmarks/linux_deadline_recovery_2026_10_06/verified-native.json。Clippy -D clippy::all通过但21条既有Rust警告未消除；严格warnings、前端有限退出和全平台生产父项仍未完成。

## 完整工作区门禁复核

4255fc8 的本机完整工作区回归实际运行111个顶层目标：1516通过、431失败、23忽略，33目标失败；按每个Cargo目标最后的结果计数，未将隔离子夹具的内层结果重复累计。GitHub同提交Linux/Windows严格Build失败，原始日志记录dead-code错误，macOS相关任务仍排队。专项门禁不能抵消该全量失败。测试专用LinuxScanImageError只应在test配置编入；历史Rust pre_exec安装方法无调用者，当前生产seccomp保持由原子出生C路径使用同一BPF program安装。移除闲置入口不完成严格质量或生产父项。

原全量运行中未提供独立协议驱动夹具，造成5项原owner错误处置用例失败。通过当前Cargo example明确生成夹具并按实际artifact注入DISKGRAPH_SCAN_DRIVER_FIXTURE后，该组6/0/0通过。CI新增相同构建/摘要/来源绑定步骤；只供测试，不设置产品镜像环境值，不改变安装信任、请求权限或默认扫描Unsupported状态。严格Build仍有未完成项，不能以该局部复查抵消原431项失败。
