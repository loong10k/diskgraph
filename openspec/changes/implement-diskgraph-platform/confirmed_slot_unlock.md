# 完整退休确认后的显式容量解锁

保持原退休门禁：封闭出生、独占销毁 Engine、原扫描与探测恢复池全部确认、原期限尚有效。随后在原 held File 内同步回读 CLEAN，显式 unlock 成功后立即消费 ActiveSlot。禁止在成功 unlock 后追加可能返回可重试错误的期限检查或读写；旧 owner 不再触碰新 owner 的记录。

CLEAN 写入、同步或回读失败仍返回原错误且保留原对象，不执行 unlock。Pending、外部 Engine 引用、恢复失败和异常 Drop 不解锁或写 CLEAN。unlock 错误保留原对象且不报告完成；不超出 OS 合同声称所有错误都证明锁仍存在。

真实 Unix fork 回归在独立测试进程中运行：ACTIVE 时新 File 必须 Busy；原确认与关闭后，继承句柄的子进程仍等待管道时新 owner 必须能认领并写 ACTIVE。旧代码实际 RED 为 Busy，新代码 GREEN。测试最初直接 fork 并发 runner 导致其它异常 Drop 测试的锁被继承，现通过独立进程隔离，不放宽旧断言。失败也由 RAII 释放管道并 wait 原子进程。

本机 active_slot_tests 2 passed，supervisor_binding_tests 9 passed。完整 Unix/Windows 原生 CI 仍待验收；本修改并不证明全量测试偶发 Busy 的原始进程来源已定位，也不实现独立后台监督或 Windows 持久资源池部署。

夹具观察：外层实际 Child 保留原 owner，5 秒 try_wait 期限，失败 kill/wait；stdout/stderr 分别最多读取16KiB。内层释放后2秒WNOHANG观察，超时终止并实际回收；kill 后回收依赖OS，不宣称严格墙钟。新 owner 认领后且释放管道前，原PID的WNOHANG必须返回0；释放后必须确认exit0，防止提前退出造成假绿。最终slot测试2项通过，Engine Clippy及fmt通过，source_layout6项通过；同提交原生平台CI仍待验收。
