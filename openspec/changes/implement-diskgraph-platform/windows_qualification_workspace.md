# Windows 原生验收的完整 workspace 隔离

本项延续 15.13 原生证据门禁，不完成父项。

## 验收要求

对旧版对照、新版候选和普通退出基线分别导出冻结 manifest 指定的完整 Git commit，再覆盖逐文件摘要校验的现有候选归档。不得将旧 Cargo.lock 写入当前产品 checkout，不得去掉 --locked 或改写归档来掩盖依赖差异。原目标 RED、65 个候选用例与结果核验保持不变。CI 先取得精确历史 commit，再执行所有隔离测试。

## 证据

CI 37410205333 在旧版对照编译阶段实际失败，错误为 cannot update the lock file；新版用例跳过，不计通过。新增隔离回归先 RED 后 GREEN；三份完整导出均通过真实 cargo metadata --locked 全依赖解析。分别注入当前 CLI manifest 后均真实复现锁文件拒绝。归档逐文件摘要保持一致，产品 checkout 未修改。Windows 驱动 22 项和挂载保护 13 项本机通过。日志见 docs/benchmarks/windows_workspace_isolation_2026_10_06。

该记录不证明 Windows 原生编译、65 项运行或全平台生产就绪；等待新的真实 CI。

## Windows 换行转换的真实回归

CI 37411176883 在驱动单元测试发现完整导出受 core.autocrlf=true 影响，与原 Git blob 的 LF 字节不一致。保留逐字节断言，本机通过真实 Git 环境设置复现三份快照 RED。导出命令显式关闭 autocrlf 并设 core.eol=lf 后，三份精确源码及完整依赖/混合 manifest 反例均通过，驱动22和挂载13通过。原始 CI 与 RED/GREEN 日志见 docs/benchmarks/windows_archive_eol_2026_10_06；不计原生用例通过。
