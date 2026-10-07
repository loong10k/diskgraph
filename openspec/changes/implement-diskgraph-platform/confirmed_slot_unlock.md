# 完整退休确认后的显式容量解锁

保持原退休门禁：封闭出生、独占销毁 Engine、原扫描与探测恢复池全部确认、原期限尚有效。随后在原 held File 内同步回读 CLEAN，显式 unlock 成功后立即消费 ActiveSlot。禁止在成功 unlock 后追加可能返回可重试错误的期限检查或读写；旧 owner 不再触碰新 owner 的记录。

CLEAN 写入、同步或回读失败仍返回原错误且保留原对象，不执行 unlock。Pending、外部 Engine 引用、恢复失败和异常 Drop 不解锁或写 CLEAN。unlock 错误保留原对象且不报告完成；不超出 OS 合同声称所有错误都证明锁仍存在。

真实 Unix fork 回归在独立测试进程中运行：ACTIVE 时新 File 必须 Busy；原确认与关闭后，继承句柄的子进程仍等待管道时新 owner 必须能认领并写 ACTIVE。旧代码实际 RED 为 Busy，新代码 GREEN。测试最初直接 fork 并发 runner 导致其它异常 Drop 测试的锁被继承，现通过独立进程隔离，不放宽旧断言。失败也由 RAII 释放管道并 wait 原子进程。

本机 active_slot_tests 2 passed，supervisor_binding_tests 9 passed。完整 Unix/Windows 原生 CI 仍待验收；本修改并不证明全量测试偶发 Busy 的原始进程来源已定位，也不实现独立后台监督或 Windows 持久资源池部署。

夹具观察：外层实际 Child 保留原 owner，5 秒 try_wait 期限，失败 kill/wait；stdout/stderr 分别最多读取16KiB。内层释放后2秒WNOHANG观察，超时终止并实际回收；kill 后回收依赖OS，不宣称严格墙钟。新 owner 认领后且释放管道前，原PID的WNOHANG必须返回0；释放后必须确认exit0，防止提前退出造成假绿。最终slot测试2项通过，Engine Clippy及fmt通过，source_layout6项通过；同提交原生平台CI仍待验收。

补充旧 owner 无干扰回归：新 owner 写 RESERVED 与 ACTIVE 后分别再次 poll 已退休 owner，记录必须保持不变；监督绑定9项通过。带实际本轮编译驱动的当前 dirty worktree Engine 全量为531通过、98失败、13忽略；98个失败全部为 Business(Unsupported)，不作全量通过声明。驱动12项及原失败资源处置6项均执行通过，详见 engine_01e450e_actual_driver_2026_10_07.json；夹具不是产品扫描镜像。

## 原管道 panic 关闭观察隔离

完整并行 Engine 回归出现一次原 pipe 数字FD EBADF断言失败，单独及完整复跑通过。测试在清理后观察裸数字，其他线程可在该观察前复用数字，不能据裸数字仍有效推断原pipe泄漏。改为重新执行本测试的独立单测试子进程，保留原panic载荷、两端精确EBADF和poison后真实新出生/清理断言。父进程5秒轮询期限、RAII kill/wait、最多16KiB失败诊断；不宣称OS wait硬抢占。此变更仅隔离测试观察，不修改生产FD清理、不代替三平台生命周期门禁。

本机隔离验证：出生协调组8/0，source_layout6/0，fmt与Clippy通过；完整Engine534/98/13，98失败全部Unsupported。双路审查APPROVE/CLEAR。增加子进程必须实际执行1项成功测试的输出断言，额外exact1/0通过。不能确认此前偶发失败根因，仍需目标平台并发回归。
