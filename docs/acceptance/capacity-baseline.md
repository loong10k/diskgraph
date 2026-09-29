# P7 容量/延迟基线（任务 8.11）

日期：2026-09-29 · 主机：macOS（darwin 27.0.0，arm64，Apple Silicon）· 执行：`cargo test -p diskgraph-engine --test capacity_baseline -- --ignored --test-threads=1 --nocapture`

结果 3/3 通过。断言只证明**规模下的正确性**（完成且精确），以下数字是**记录基线**，不是门限——劣化时是排查线索，不是失败门禁。

| 场景 | 规模 | 实测 |
| --- | --- | --- |
| 宽平目录索引（Q-02） | 20,000 文件单目录 | fixture 构建 1.84s；索引发布 1.18s；top-100 子节点查询 1,130µs；文件数精确 20,000 |
| 历史保留（RT-04/ST-04） | 同 scope 8 个 revision | 8 轮索引合计 0.21s；最新 revision 可完整加载 |
| 海量重复疑似组（CT-03） | 5,000 同大小文件 | 元数据分组 50ms 产出 1 组 5,000 成员；仅元数据、零内容读取 |

## 口径与限制

1. **数字不外推。** 本基线是本机一次运行的观测；不同硬件/负载下会不同。回归判断应比较同机趋势，不跨机比较绝对值。
2. **预算正确性优先于速度。** 全部路径在预算与身份契约内运行（索引受 `ScanBudget`、查询受页界、分组零内容 IO）；没有为速度绕过任何正确性门禁。
3. **未覆盖。** 高扇出**关系**（边查询）与内容哈希作业的规模数字留待 8.4 的授权哈希作业接入 CLI/MCP 后补充；本机为 APFS，冷/热缓存差异未分离。

## 8.9 / 8.10 平台矩阵留档（未勾选，如实说明）

- **8.9 Windows 原生语义**（卷/file ID、reparse point、Recycle Bin）：本机为 macOS，无 Windows 主机可验证。已登记 `diskgraph_testkit::real_os_requirements()` 的 `WindowsNativeSemantics` 条目（FS-02 / PF-03）。通用语义中平台无关的部分已在库中成立：`growth` 历史比较在 `volume_id` 缺失时拒绝（身份不足不产出可信比较）。
- **8.10 三平台回收/权限/占用/复制保真矩阵**：同因留档，登记 `LinuxDesktopMatrix` 条目（OP-06 / RE-04）。写适配未通过验证前保持禁用（ops 层 purge 默认关闭即为该姿态的体现）。
