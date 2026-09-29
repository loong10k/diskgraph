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


## 2026-09-29 性能优化轮（v4 结构化列 + 窄读路径）

真实 4.3–4.5M 节点 home revision，`diskgraph tree --depth 3`（release，单 CLI 调用）：

| 版本 | 耗时 | 变化 |
| --- | --- | --- |
| 优化前（serde_json 全量解析 + 默认 PRAGMA） | 23.8s | 基线 |
| +WAL/synchronous NORMAL/mmap、as_bytes 读取、批量 8192 | ~22.7s | −5% |
| +schema v4 结构化列（load_revision 零全量 JSON 解析） | ~21.5s | −10% |
| +tree 专用窄读（零 JSON、零 locator、免 DiskGraph 中间层） | **10.1s** | **−58%（2.4×）** |

实验后**撤回**的：simd-json——实测 4.3M 短文档（~500B）上比 serde_json 慢 2 倍（每调用常数开销大于 SIMD 收益）；该结论写入 store 层注释。并行 parse（rayon 16k 行块）保留于 load_revision 路径。

同步修正：`diskgraph index` 发布侧因 v4 每行多写 10 个结构化列，430 万节点全量扫描 4m17s → 5m58s（+39%，一次性写入成本，换读侧 2.4× 持续收益）。全部渲染输出对账 IDENTICAL（v3 回退路径与 v4 快路径输出逐字节一致）。
