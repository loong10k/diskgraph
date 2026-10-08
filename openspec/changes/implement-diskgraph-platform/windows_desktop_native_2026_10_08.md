# Windows 台式机原生验证实录

固定源码 1ac266a0c7b13e7da1eb31f4376538aa5efb00bd，实际目录 E:\workspaces\workspace-loong10k\diskgraph，Rust 1.99.0 x86_64 MSVC。运行前后 HEAD 相同且工作区干净。证据原件保存在台式机 C:\Users\hnxyh\AppData\Local\Temp\diskgraph-desktop-jetfdyj0；当前仓库只保存可读取回执与结果摘要，不声称已下载被工具拒绝导出的归档。

## 已执行结果

| 验证 | 结果 | 边界 |
|---|---|---|
| workspace/all-targets 构建 | 通过，123.078 秒 | 不代表测试通过 |
| 撤权 / 能力期限 / 关系定向组 | 6 / 14 / 17 项通过 | 不能替代并发门禁 |
| workspace/all-targets 默认并发 | 2148 通过、18 失败、25 ignored；646.828 秒 | 133 个 stdout 顶层结果汇总，不重复计 stderr 内子测试 |
| Store 串行 | 359 通过、0 失败、7 ignored；测试 163.62 秒 | 原生 Windows |
| workspace/all-targets Clippy，-D warnings | 通过 | 37.828 秒，保留原警告门禁 |
| 真实 Cloud Files | 1 通过、0 ignored | 产品 fetch=0，普通读取正控 fetch>0；原生标记存在 |
| Python 3.11 全脚本 discover | 282 项，11 failures / 31 errors / 7 skipped | 不能算通过；含平台限定脚本、缺失身份 API、换行摘要和符号链接权限错误 |
| Python 3.13.12 包组成 | 10 通过、1 失败 | 链接夹具创建 WinError 1314，未跳过 |
| Python 3.13.12 manifest 准入 | 3 通过、1 error、1 Unix FIFO skipped | error 同为链接创建权限 |
| Python 3.13.12 Windows Job 回收合同 | 8 通过 | 含真实后代超时回收 |

第一轮 Engine 为 669 通过 / 18 失败 / 4 ignored：11 项因本次启动漏设 DISKGRAPH_SCAN_DRIVER_FIXTURE；6 项 Git 探针超时；1 项历史末段期限失败。已补充 WD-02 的真实 Cargo example 产物绑定。

随后保持同一 SHA、默认并发、原始期限与断言重跑整个 Engine lib：674 通过 / 13 失败 / 4 ignored，159.85 秒。12 项 driver 测试全部通过；仍有 12 项 Git 相关超时及 1 项后代心跳标记不存在。第一次期限失败在第二次通过不等于已修复，不把不同运行的成功项拼接为完整通过。详情和真实产物摘要见 docs/benchmarks/windows_desktop_1ac266a0_2026_10_08/driver_summary.json。

## 环境与剩余门禁

PATH 中默认 Python 为 Espressif 3.11.15；进一步定位机器已装 uv 管理的 Python 3.13.12，后续明确使用其绝对路径，未安装软件。运行账户缺少创建符号链接的权限；未启用开发者模式、提升权限或把失败改成跳过。精确 MSRV 1.97.0 本机仍未执行。

Release 构建通过（55.985 秒），固定旧版 e4d6074d 独立构建通过（64.375 秒），实际 worker 绑定与归档通过。包验收在 stdio 后的 HTTP 阶段失败：按宿主 GBK 解码 Rust UTF-8 错误信息产生 UnicodeDecodeError，继而出现 None stderr 拼接异常。运行 69.093 秒，升级/回滚和 20k/200k 尚未执行，不能标成性能超时。回执及四个实际产物 SHA256 见 release_summary.json。

新增真实子进程回归强制模拟 GBK 默认编码，原实现两个用例均因 UnicodeDecodeError 失败；明确设置 Rust 输出与 JSON 管道的 UTF-8 后两项通过。HTTP 对 whoami/icacls 的本机命令仍保留原生代码页，只固定产品进程及其日志编码。包编排给 Python 子脚本明确 PYTHONIOENCODING=utf-8；不改变任何请求、扫描或回收期限。HTTP soak 合同 18 项、包组成 11 项在本机通过，Windows 修复后实际包验收另记。

Git 原默认 15 秒、授权原 50ms 及正式包 300 秒门槛不变。整体生产就绪与 OpenSpec 归档继续保持未完成。

## 9ade861e 台式机后续验收

远程快进到 9ade861e331366e7fd5279940118fe9ba4d5ec79 后，按 HEAD blob 验证并备份旧 CRLF 工作副本，重新物化 1588 个应为 LF 的文件；Git 内容与索引最终均干净。没有修改 vendored 字节。实际子进程 UTF-8 两项与检出字节两项通过。

Windows PowerShell 5.1 实际执行发现两个 UTF-8 无 BOM 脚本解析失败（2 失败 / 1 通过）。仅加 BOM 的临时候选重跑为 3 通过；修复已提交 13365516，但本次产品二进制仍固定 9ade861e。包准备使用该已验证 BOM 候选脚本，必须保留此混合证据边界。其 SHA256 与提交内容一致，详见 powershell_bom.json。

原生 Release 构建通过（68.360 秒），真实 worker 绑定通过。包验收顺序执行 stdio、HTTP、固定旧版升级/回滚、20k，再在 200k 失败。完整包命令 466.344 秒，不将前面成功项拼成全包通过。

| 实际验证 | 结果 |
|---|---|
| HTTP | 14/14；60.00086 秒内 1547 请求，p50 42.2221ms / p95 49.9592ms，含匿名、恶意 Origin、未授权、撤权与 legacy SSE |
| 20k 完整负载 | 6/6；20001 节点、32 查询、4 客户端；造文件 9.102 秒，扫描 72.011 秒，清理 10.826 秒；查询 p50 61.317ms / p95 75.461ms |
| 20k DB + WAL | 54,996,992 字节 |
| 200k 完整负载 | 失败；造文件 127.8285629 秒，随后扫描未结束即触发原 299.9831243 秒整体期限；没有完成查询和清理结果 |

随后仅作成本定位，保持同一二进制、原始扫描与安全检查，另跑开启诊断的 20k（不是重跑200k或替代其门禁）：6/6，整次 98.563 秒，扫描 73.057 秒，根链累计校验 55.715 秒、40003 次、8 个句柄；观测和编码共 60.410 秒，staging 写入 0.547 秒。根链耗时包含在观测耗时中，不相加，也不据此断言内核、杀毒软件或 CPU 是根因。完整阶段记录保存在 scan_cost_result.json；生产根校验仍原样保留，性能问题未修复。

原件目录 C:\Users\hnxyh\AppData\Local\Temp\diskgraph-desktop-jetfdyj0，当前仓库回执位于 docs/benchmarks/windows_desktop_9ade861e_2026_10_08/。结束后 HEAD 仍为 9ade861e、工作区干净，按本轮产品与证据目录筛选的残留产品进程为零（retirement.json）。这不是全机器进程审计。未启用符号链接权限、未安装 MSRV、未放宽任何期限；全平台最终同源码验证仍未通过。
