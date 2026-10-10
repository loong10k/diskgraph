# Current native CI qualification / 当前原生 CI 验收

Tested source: `2e5199459c1aacfb07ee93fc0827e02e3918f100`.
Workflow: https://github.com/loong10k/diskgraph/actions/runs/38049846922

The terminal API records all 23 jobs as successful. The macOS Intel history budget suite ran 10 tests with no failures or ignored tests; the Windows MSRV FFI library ran 104 tests with no failures or ignored tests. Raw logs are retained in gzip form alongside focused extracts and SHA-256 receipts. Five native package artifacts were separately checked against their archive checksums and actual worker bytes. Their package evidence lives in the matching `*_2e519945_2026_10_10` directories.

终态 API 记录全部 23 个任务成功。macOS Intel 历史预算目标为 10/0/0，Windows MSRV FFI 库为 104/0/0。压缩原始日志、目标片段及 SHA-256 回执保存在此；五种原生包的归档摘要和 worker 内容另在对应目录核验。

These observations qualify this run and exact source. They do not prove that the preceding native timing failures cannot recur. Original failures remain in `readonly_ci_ba2_2026_10_10`. The release RSS observation used the ba2 package, not this source; it is not a strict memory limit or a paired performance improvement. ABI inspection is not execution on glibc 2.17 userland. Unsupported write actions, real cloud-provider non-materialization, mobile-host integration and finite frontend exit under unretirable I/O remain outside this evidence.

本记录只证明该提交本轮验收，不代表旧偶发时限失败根因已消除。ba2 原失败及 RSS 证据分别保留，不把它们改记为本提交的实际执行；ABI 检查不等于旧 glibc 用户环境运行。不支持的写操作、真实云 provider 禁止物化、移动宿主集成和不可回收 I/O 下有限退出仍需各自验收。
