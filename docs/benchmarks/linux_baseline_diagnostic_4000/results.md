# 原 Linux 配对性能失败的独立诊断

run37549794669，原 artifact11452877584。200k 配对旧基线侧失败后，独立 diagnostic-only 重扫执行成功 exit0，未产生诊断 stderr。重扫使用新隔离夹具，并修改四个原错误丢失点仅用于诊断输出；其时序及产物均不能计入十二次配对测量或性能比率，不能视为故障已解决。

本轮逐一核验原 receipt 的 before/after/harness 源码 SHA256、各命令原 stdout/stderr 长度与 SHA256。归档保留原日志、原 receipt、实际源码与差异；仅不重复存储 baseline-archive.stdout 的源码 tar 字节流。原下载 ZIP 的 SHA256 和完整本机路径保存在本目录 receipt.json，不修改原原始日志。

当前结果缩小了诊断范围：该独立观测没有捕捉失败原因。原 paired failure 门禁保持未通过，不换基线、不省略失败一侧、不用重扫填补性能数字。原监督进程/CLI/MCP 接入及全平台验收继续未完成。
