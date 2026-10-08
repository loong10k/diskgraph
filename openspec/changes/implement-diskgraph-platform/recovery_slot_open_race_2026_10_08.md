# Unix 恢复槽并发打开的实际红绿回归

旧1d434c9在两套macOS ARM CI的不同CLI场景返回原openat ENOENT，诊断同为held_path_same/regular。本机新增四个独立真实进程对同一0700隔离域认领原槽的测试，旧实现实际失败：原目录身份未替换，槽项仍普通文件，O_CREAT路径返回ENOENT。尚未证明具体内核内部原因，不把它归为父目录被删除。

修复把已有槽以O_RDWR/NOFOLLOW/CLOEXEC/NONBLOCK普通打开；仅ENOENT时尝试O_CREAT|O_EXCL，若EEXIST则在原目录句柄中普通重开一次。总原生打开次数最多三次，每次检查原deadline。原文件owner/0600/单链接/普通类型、原锁、原状态及目录sync均保持。不广泛重试ENOENT、不修复目录、不清除异常记录，不重新签发身份或容量。

两轮每轮四进程共2048次实际认领/abort通过，13.55/14.37秒。最终仓库测试每进程128次、四进程512次，2.97秒通过；减少的是重复次数，实际调用链、并发进程数、原断言及期限均保持。父项1/0/1（ignored子夹具由父项实际启动四次，各精确1/0/0并核验完成标记），并非跳过真实进程。目标域7/0/1、原错误诊断3/0、源码结构6/0、全目标Clippy、fmt和OpenSpec严格验证通过；已删除held目录仍精确ENOENT且不重建，链接/多硬链接/权限/异常ACTIVE拒绝保持。

原始日志及最终源码摘要在docs/benchmarks/recovery_slot_open_race_2026_10_08。macOS Intel、Linux原生新源码尚未执行；本机Docker返回元数据库只读文件系统，容器未创建，不计作Linux测试通过。该修复不关闭产品broker监督链、有限前端退出或整体生产门禁。当前CI37750244950尚有三个原生测试job运行，不为推送取消它们。
