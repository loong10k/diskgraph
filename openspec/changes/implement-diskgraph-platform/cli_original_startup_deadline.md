# CLI 原启动期限提交门禁

沿用 frontend_recovery_supervisor.md 与 PF-06，不新建变更。

`CliEngineHost::open` 只生成一次原30秒期限，再委托 `open_until`。进入时已过期返回精确 BudgetExceeded，不创建数据目录。原期限传入扫描镜像准入；Engine 构造完成后须经过私有 `finish_open_until`，迟到时消费原宿主，经原 execute 封闭准入并保留原恢复责任，不返回可用宿主。

本机验证：CLIHost 三项测试通过，CLI all-target Clippy -D warnings 通过。负对照将迟到分支临时替换为直接返回错误，原 registry 封闭回归实际失败（得到 Unsupported 而非封闭后的 Conflict）；恢复正式实现后三项重新通过。代码复审 APPROVE、架构复审 CLEAR。

边界：同步数据库构造不能硬抢占，迟到分支的数据库可能已经出生；测试直接触发实际原宿主提交门禁，不模拟整个构造耗时。恢复夹具无活跃 child，Windows探针夹具不证明原探针池绑定。既有 execute 仍可能无限等待；可信监督、全局容量、有限前端退出与公开EOF尚未接通，父生产任务不勾选。
