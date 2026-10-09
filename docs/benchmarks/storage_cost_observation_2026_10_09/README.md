# 负载存储成本 / Load storage costs

`accept-readonly-load.py` 原先只报告查询后的数据库与 sidecar 合计。
本轮保留 `database_and_wal_bytes`，增加 `storage_cost`：范围注册后
`before_scan`、扫描发布后 `after_scan`、查询后 `after_queries`，以及
`scan_delta` / `query_delta`。每项分别含数据库、WAL、SHM 和合计字节。

仅统计隔离目录中的 `diskgraph.sqlite` / `diskgraph-control.sqlite`
及各自 `-wal` / `-shm`。读取失败和非普通文件必须报错；缺失 sidecar
合法计为零。增量有正负，checkpoint 缩减不得夹为零。这是各时点的
文件长度，不是卷分配量、累计写入、峰值 WAL 或 RSS。

真实 SQLite 回归验证 WAL 写入增长、TRUNCATE 后缩减；额外覆盖两库
选择、错误传播及主报告接线。本机 46 项、1 项跳过，原生 Windows
和完整二进制负载未运行；不得据本报告关闭平台、内存或长期运行门禁。
原时间门禁、四客户端、覆盖和回收检查均保留。详见 [receipt.json](receipt.json)。

The load harness retains `database_and_wal_bytes` and adds `storage_cost`
observations after scope registration, scan publication and concurrent queries.
Each observation separates database, WAL and SHM file lengths. Signed scan and
query deltas preserve checkpoint shrinkage. Only the two fixture databases and
their sidecars count; unreadable or non-regular entries fail the observation.

These measurements describe file lengths at three boundaries, not allocated
volume space, cumulative writes, peak WAL or RSS. Real SQLite tests verify WAL
growth and truncation. The local Python suite ran 46 tests with one skip;
native Windows and the packaged load were not executed. Existing deadlines,
four clients, exact coverage and retirement checks remain unchanged.
