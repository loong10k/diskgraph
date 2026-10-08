# 已退出命令与延迟清理的错误保留

范围：EC-02/EC-04，来自 CI37731786365 Windows stable 的 stash 损坏回归。命令已经正常非零退出、双流收齐，但清理 Pending 时执行器只返回 cleanup 错误，丢弃了命令退出状态。

实现：共享结果合并函数在清理失败时保留正常非零退出码、已扣费 stderr 和原清理错误。stderr Vec 转移到 Arc，不因预算锁存错误再复制正文；失败 stdout 不保留。Git 可信库诊断使用类型化失败投影，保留既有 git exit status 前缀、stderr 和全部清理次因；产品入口仅映射有界 Unavailable，不导出原始 stderr。Unix 与 Windows 合并入口一致，原预算取消/期限优先级和 Windows owner 移交逻辑保持。零退出且清理失败仍不得返回成功输出，已经清理的非零输出保留原 Git fallback 语义。

先红：三项回归中目标用例因只得到 probe unsupported: original owner retained 失败，另外两项正控制通过。修复后探针29、真实本机 Git stash15、私有目录清理5、源码规范6均通过，合计55项；Engine all-targets Clippy、格式及 OpenSpec 严格校验通过。证据见 docs/benchmarks/probe_exit_cleanup_2026_10_08。

该修复不证明原生进程已经退休，也不释放仍 Pending 的容量、删除仍借出的目录或延长原期限。当前远端 CI37735499585 在 f9e0a39 上运行，不包含此候选；原生 Windows 同提交复验待完成。正式生产父项继续开放。
