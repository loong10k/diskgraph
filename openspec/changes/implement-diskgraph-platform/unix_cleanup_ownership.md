# Unix 清理失败的所有权与容量语义

本项属于 15.13 原生回收门禁，不完成该父项。

## 验收场景

真实子进程退出、管道 EOF 后，被宿主外部 waitpid 消费原 leader。原 owner 检测到 ECHILD 后必须拒绝基于旧 PID/PGID 的清理，后续重试也不得报告成功。转入恢复 registry 后，每次 drain 必须保留原失败及已占用槽位，不允许新的 reservation 复用该容量。

## 当前证据

既有真实外部回收测试加强后 RED：第二次 cleanup 错误返回成功。移除丢失等待权分支的 cleaned=true 后 GREEN；三次直接重试与三次 registry drain 均保留失败及容量。macOS 原生子进程回归 40 通过、0 失败，唯一默认忽略的隔离辅助夹具由真实测试显式执行。原始日志和源码摘要见 docs/benchmarks/unix_lost_wait_ownership_2026_10_06。

本项不证明 Linux 原生执行、首次检查之后的 wait 竞态、有限时间回收或全平台生产就绪。

## 后续末段 wait 竞态

原先的完成分支还会在末段 wait 报错后置 cleaned=true。既有回归增加初检之前和末段 wait 之前两个真实外部回收时序。末段使用仅测试编译的线程局部单次 checkpoint 调用真实 waitpid，不模拟 ECHILD。原代码实际 RED；修复后 wait 的 ECHILD 撤销旧数值身份权限，保留原错误及容量，其他 wait 错误保留 owner 重试。macOS 40/0/1、结构6/0通过，日志见 docs/benchmarks/unix_cleanup_late_wait_2026_10_06。进程组终止失败路径、Linux 和有限时间回收仍未完成。
