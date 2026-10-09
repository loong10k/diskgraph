# Windows 台式机补充验收（2026-10-09）

实际目录：`E:\workspaces\workspace-loong10k\diskgraph`。基础提交为
`f07f463b053484c404794def7d4194c8af7abd31`；授权回归使用提交 `8bd76e5a`
对应的四份源码，完整 SHA-256 清单见授权 receipt，不把基础 HEAD 当作修改后源码身份。

实际执行结果：

- policy capture 回归：3 passed / 0 failed / 0 ignored。
- grouped terminal SQL 回归：1 passed / 0 failed / 0 ignored。
- Engine 全目标 Clippy，`-D warnings`：exit 0。
- 用已有 MSVC 编译两份真实 C ExitProcess 产物，随后执行原产物和实际 CLI 转交：
  `serve_windows_exit_status` 2 passed / 0 failed / 0 ignored。

全部原始日志、执行回执及 C 产物构建回执已完整下载，以确定性 gzip 保存。
`evidence_hashes.json` 记录远端原路径和未压缩内容 SHA-256；下载后独立解压核验。
远端授权作业 `wc_job_5BDNabsIIt19VpV4` 与退出状态作业
`wc_job_8bhPYnv9WMyXeJJt` 均已实际终态 exit 0。

编译器记录一次增量缓存目录访问被拒绝警告；实际编译、测试和 Clippy 均完成，
未删除缓存或更改权限以隐去该观察。

这六项回归不代表完整 Windows 工作区、200k 性能或生产就绪通过。
之前完整工作区的失败记录保留在 `windows_workspace_f07f463b_2026_10_09`。
后续完整工作区作业 `wc_job_XrZZ-5E7upk15cp8` 使用新构建并独占复制的实际
scan worker、显式 driver 夹具和两份真实退出状态夹具；不改变默认测试并发、
产品 SQL 期限或测试断言。该作业尚未取得终态，此处不计通过。
