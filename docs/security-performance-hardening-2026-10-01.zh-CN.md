# DiskGraph 安全修复与性能优化验收 — 2026-10-01

[English](security-performance-hardening-2026-10-01.md) · [简体中文](security-performance-hardening-2026-10-01.zh-CN.md)

沿用 [implement-diskgraph-platform](../openspec/changes/implement-diskgraph-platform/) 作为唯一规格事实源。保留原 README、架构图和 locator 修改，不创建或切换分支，不提交、推送、发布，不迁移真实数据库或安装宿主配置。验证全部使用隔离数据库/文件夹具，vendor 的 pin、源码与摘要保持不变。

## 已实现与回归覆盖

| 范围 | 实现及证据 |
| --- | --- |
| 请求身份 | 不可变上下文记录主体、token 能力、签发方、传输与到期时间；issuer + subject 映射 SHA-256 主体。token 能力与实时数据库授权取交集。HTTP/现代 SSE/legacy SSE 在分发前校验 Origin 和认证；远程启动不创建或继承本地管理员权限。二进制 loopback 也必须配置认证。 |
| 长连接 | legacy session 绑定主体；两种 SSE 共用每主体 4 条配额，按秒核验 token 到期与实时撤权并关闭连接。真实 socket 覆盖双主体、能力/授权交集、撤权与到期。 |
| revision 归属 | schema 5/6 持久化 server/scope 与规范化搜索字段；实际归属统一授权，传入 scope 无法替代。旧根只在唯一匹配 scope 时回填；非 UTF-8 显示碰撞等歧义保持未绑定并拒绝访问。CLI/MCP/旧 FFI 读前进入 Engine。 |
| 内容预算 | 每次读扣减剩余预算，chunk 最大 64 KiB，分配受限；不完整哈希返回空摘要和未确认状态。Unix 从根/父目录句柄逐组件 no-follow 打开，核验 fd 身份、长度、纳秒 mtime/ctime，读取中检查实时授权；无法可靠验证的平台拒绝。 |
| 文件安全 | 父目录及文件句柄固定，独占 staging，原子禁止覆盖发布；复制核对双侧摘要与源稳定性，保留已验证 staging/source 句柄至发布/删除。macOS 核验 mode、mtime、flags、ACL、xattr；覆盖原地改变、父目录替换、目标竞态、staging 篡改和源安全移除。 |
| 窄读及分页 | MCP node/top/children/explore/search 窄读。Rust Unicode 小写子串匹配保持兼容，参数化 SQL 与稳定排序；搜索 keyset 游标绑定主体/scope/revision/过滤/实际排序/策略版本，旧游标拒绝。显式 offset 保留，目录页仍兼容 offset。 |
| 查询预算 | 独立 SQLite 读连接、有限缓存、progress 期限/取消接口，串行图库写连接；不再全局锁住工具执行。树按层读取，有序 SQL 迭代器合并历史，返回节点/时间/响应字节截断。SQL 超时保留局部结果，`complete=false`、`summary_is_partial=true`；总数未完成时 `node_counts_complete=false`，不能解释为零节点。 |
| 扫描与发布 | 上游 progress 每 20 ms 检查节点/时间/撤权/取消，超限禁止发布；转换迭代处理并减少克隆。staging 按编码 JSON 与规范化字段计费，不按源文件大小计费；分批写入，发布 SQL 从 staging 生成正式节点。 |
| 容量与任务 | 入队/批次/发布检查实际目录占用和卷剩余空间（64 MiB 保留）；配额/合并在 Immediate 事务内按真实主体执行，并重查授权。严格条件认领（同名 owner 也不能接手未过期任务）、每代次独立取消标志、递增 fencing、30 秒租约、5 秒续租贯穿转换/staging/collector，失效 owner 不能写入、发布或完成；重认领采用新命名空间并重扫。 |
| 回收与迁移 | `snapshots prune --scope S --keep-last N` 默认预览，`--apply` 才逻辑删除。保护 latest/pin/操作/恢复引用，旧引用不精确时保留整个 scope。迁移前 SQLite 一致性备份包含已提交 WAL；图库 WAL/NORMAL，控制库 FULL。 |

底层 raw store 和旧本地签名作为可信兼容入口保留，在架构契约中标记弃用；远程必须走授权服务。显式可信的 in-process HTTP 测试/兼容入口仍保留，`open_remote` 与 server 二进制不允许未认证访问。systemd 示例现在要求操作员配置认证参数。

## 测试证据

先验证目标失败，再修复至通过：无权限 token、恶意 Origin、A scope 读 B revision、撤销后发布、哈希超预算、容量拒绝仍入队、原地修改、WAL 备份、窄读、租约失效、同名 owner 新代次/未过期拒绝、过期代次取消标志隔离、跨主体合并、无授权操作计划、SQL 超时，以及远程复用本地数据库的未认证路径。

持续保留的测试入口：

- [Engine hardening](../crates/diskgraph-engine/tests/hardening.rs)：预算、撤权、容量、大稀疏文件元数据计费、tree/history 截断、旧归属歧义。
- [MCP hardening](../crates/diskgraph-mcp/tests/hardening.rs)：真实 socket 多主体读取/SSE、撤权/到期、Origin、issuer、远程/本地身份隔离、无关损坏行证明窄读。
- [job_claim](../crates/diskgraph-store/tests/job_claim.rs)：两个真实子进程竞争、过期 owner、fencing、实时入队授权及主体合并。
- [migration_backup](../crates/diskgraph-store/tests/migration_backup.rs)、[CLI](../crates/diskgraph-cli/tests/cli_flow.rs)、FFI 单测与 [Ops](../crates/diskgraph-ops/src/tests.rs)：WAL/迁移、预览及引用保护、兼容授权、目标/父目录竞态、摘要和原生 ACL/xattr 保真。

完整门禁计数在最终执行后附于文末。常规 ignored 的真实项目、真实卷和宿主测试仍为未执行，不计为通过。

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
# fmt 使用明确 workspace package，排除 vendored path dependency
openspec validate implement-diskgraph-platform --strict
DISKGRAPH_BENCHMARK_OUTPUT=measurements.json cargo test --release --offline \
  -p diskgraph-engine --test hardening_benchmark -- --ignored --nocapture
```

## release 前后实测

[原始 JSON](benchmarks/hardening-2026-10-01.json) 保存环境、基线和所有分布。旧版是 `e4d6074` 的临时源码归档，无分支/checkout 变化；测试 harness 调用旧完整 revision 和树 API，fixture 依赖修改不改变基线生产代码。前后均为 release、32 字节文件、20k/200k 宽目录和 300 层深目录，每种场景独立进程，前后顺序执行。top20 为 31 次、tree 11 次、4 个并发读取各 31 次；扫描每场景一次，属于观察值，不是统计保证。

| 场景 | top20 p50 ms 前 → 后 | top20 p95 ms 前 → 后 | 4 读者最大 p95 ms 前 → 后 | tree p95 ms 前 → 后 |
| --- | --- | --- | --- | --- |
| 20k 宽目录 | 11.701 → 0.134 | 12.273 → 0.273 | 46.114 → 1.370 | 26.959 → 5.707 |
| 200k 宽目录 | 105.217 → 0.142 | 112.234 → 0.217 | 411.494 → 1.752 | 291.192 → 40.914 |
| 300 层深目录 | 0.380 → 0.129 | 0.490 → 0.221 | 1.590 → 2.185 | 0.491 → 0.485 |

旧 tree 深度 2 展示全部符合节点，新 tree 用默认 100 节点/64 KiB/深度 2 预算并明确截断；其输出量不同，表中是目标行为前后对比，不能理解为相同输出量的速度对比。JSON 另保留当前数据库上完整加载与窄读的配对控制。

| 场景 | 扫描秒 前 → 后 | 扫描阶段进程峰值 RSS MiB 前 → 后 | 数据库 MiB 前 → 后 | 结束时 WAL 字节 前 / 后 |
| --- | --- | --- | --- | --- |
| 20k 宽目录 | 0.193 → 0.450 | 37.172 → 34.484 | 17.727 → 26.980 | 0 / 0 |
| 200k 宽目录 | 1.908 → 3.369 | 269.766 → 213.719 | 176.844 → 272.004 | 0 / 0 |
| 300 层深目录 | 0.037 → 0.042 | 13.344 → 14.062 | 1.051 → 1.742 | 0 / 0 |

200k top20 p95 约提升 516 倍，扫描阶段峰值 RSS 约降 21%；但扫描时间约增 77%，数据库物理文件约增 54%。规范化字段/索引与授权、fencing、发布校验需要时间和存储，本轮不宣称扫描更快或数据库更小；300 层小目录的并发 p95 在最终测量中变慢，不能泛化为全部负载都更快。整轮 RSS 包含完整加载、树和并发控制，不能当成窄读 RSS。WAL=0 为结束 checkpoint 后样本，未测瞬时 WAL 写入峰值。观测峰值不能证明内存硬上限。

## 兼容性与未验收边界

- 新 principal 映射需要重授旧远程授权；旧搜索游标需重新查询。wire 保留已有数据字段，新增诊断；目录 offset 兼容保留。
- 旧 running job 以 heartbeat + 30 秒初始化租约，等过期后才认领；迁移失败不能启动服务。无法确认 scope 的旧 revision 需管理员重索引。
- 上游仍构造完整树，取消受目录边界/并行任务影响，可能短暂额外工作；流式扫描和严格 RSS 上限已按批准计划排除。
- prune 仅释放 SQLite 逻辑页，不保证物理文件缩小；不自动 VACUUM、删历史或清理用户文件。旧 staging、备份和空闲页仍计实际容量。
- 句柄、摘要与原子 no-replace 不等于文件系统全局事务；观察后的内容变化、回滚目标冲突仍进入显式失败/恢复状态，无法保证保真时拒绝。
- CLI/MCP 危险文件工具继续关闭。Linux/Windows 原生写、移动 provider、云占位不下载、安装后的 Swift/Kotlin 宿主/UI 未执行；macOS ACL/xattr 夹具通过只证明库内路径，不能自动启用远程写。
- SQLite progress hook 管控语句执行，锁等待另受 busy timeout 限制；不能取消任意文件 collector 或承诺严格墙钟期限。两库没有跨库原子提交；图库 NORMAL 断电可能丢最近可重建索引提交，控制库 FULL 保持更强记录持久性要求，仍取决于底层文件系统/硬件。

OpenSpec 保持 active、未归档；跨平台任务继续未勾选，本记录不代表原始平台路线全部完成。

## 全代码复审后续整改

后续工作由 OpenSpec 13.1–13.7 跟踪。impact 与关系查询按 revision 持久归属鉴权，关系按页遍历并显式报告截断。HTTP 限制请求行、头数量、头字节、请求体及绝对读取期限；SSE 活性写入有超时。HTML 报告的标记与内嵌 JSON 分别转义。CLI `serve` 转交远程认证和 Origin 配置；socket 验收夹具使用签名测试 token 与隔离授权。

操作执行实时重查 scope、动作与策略授权、计划期限、完整 SHA-256 计划摘要、源指纹及总字节预算。Copy、Move、Restore 检查目标 scope；操作资源认领在 SQLite Immediate 事务中按真实源/目标身份完成。旧计划缺少新摘要/指纹，必须重新规划。目录源和 Windows 专用命令在无法保证已验证预算或进程树期限时返回 `unsupported`。Unix 专用命令的超时覆盖仍持有输出管道的后代进程。

取消意图跨 Engine 实例持久化，在发布 fence 再检查。重认领任务使用新 fencing 命名空间，仅清理本任务的过期 staging。FFI 控制库由无损图库路径区分；旧共享控制库只有归属唯一可证才复用，否则需管理员重新索引。TUI 对宽目录每页加载 512 项，按名称排序仅作用于当前页。稀疏索引覆盖未知大小子节点计数，有序路径索引避免历史迭代临时排序。

上方性能表测于本轮后续整改**之前**。当前代码又运行一次隔离 release 夹具；箭头表示先前加固版本到本次复审版本，两次运行并非完全同一宿主负载下的 A/B 实验。详见[原始测量](benchmarks/review-followup-2026-10-01.json)。

| 夹具 | 扫描秒数 | 扫描峰值 RSS MiB | 数据库 MiB | Top20 p95 毫秒 | 有界 tree p95 毫秒 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 20k 宽目录 | 0.450 → 0.361 | 34.5 → 34.9 | 27.0 → 29.9 | 0.273 → 0.244 | 5.707 → 1.861 |
| 200k 宽目录 | 3.369 → 4.506 | 213.7 → 213.6 | 272.0 → 301.0 | 0.217 → 0.230 | 40.914 → 10.545 |
| 300 层深目录 | 0.042 → 0.041 | 14.1 → 14.0 | 1.7 → 2.0 | 0.221 → 0.150 | 0.485 → 0.322 |

本次 200k 扫描更慢、物理库更大，有界 tree 更快；亚毫秒级 p95 差异可能含噪声，不能宣称所有查询普遍提速。关系遍历每页/方向打开窄读连接，高度节点的 impact 查询仍可能增加延迟。目标数大于零的候选选择仍加载完整 revision，以保持全局排名和祖先证据语义；store 侧可用目录索引留待后续。目录分页仍使用 offset，深页会遍历前面的行。上游扫描器的严格 RSS 或墙钟上限仍不承诺。


## 最终门禁结果

2026-10-01，本机 macOS：

| 门禁 | 实际结果 |
| --- | --- |
| `cargo test --workspace --locked` | 39 个测试套件；470 passed、0 failed、12 ignored |
| vendored scanner 自身 `cargo test --offline --quiet` | 124 passed、0 failed、2 ignored |
| release 隔离性能夹具 | 显式执行 ignored benchmark；旧源码/当前源码各 1 个聚合测试通过，3 个独立场景 |
| 明确 workspace package 的 fmt check | 通过；未格式化 vendor |
| Clippy workspace/all-targets/locked，`-D warnings` | 通过 |
| OpenSpec strict | `implement-diskgraph-platform` valid |
| vendor 开工 SHA-256 对照 | 20 个非 target 源码/manifest/pin/摘要文件逐一不变 |
| Git | 原分支 main、HEAD `e4d6074` 不变；有未提交修改，无提交/推送/发布 |

任务 12.1–12.7 按上述本机范围勾选；12.8 和原跨平台/宿主未完成任务保持未勾选。后续是在对应 Linux/Windows/设备环境取得验收证据；不自动开启危险工具、安装环境或执行真实数据迁移。

## 复审后续验证（2026-10-02）

| 门禁 | 实际结果 |
| --- | --- |
| `cargo test --workspace --locked --offline --quiet` | 39 个测试套件；518 passed、0 failed、12 ignored |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | 通过 |
| 非 vendor workspace `cargo fmt --check`；`git diff --check` | 通过 |
| `openspec validate implement-diskgraph-platform --strict` | 有效 |
| `actionlint .github/workflows/release.yml` | 通过；注册表发布等待二进制矩阵 |
| 真实 loopback HTTP、签名隔离 token | 11/11 通过 |
| CLI `serve` 带远程认证启动子进程 | 独立监听成功 |
| 当前 release 性能夹具 | 20k/200k 宽目录与 300 层深目录通过；数据见上文 |
| 上游 vendor 源码、pin、摘要 | 与 HEAD 相同；专用 pin/摘要测试 4/4 通过 |

任务 13.1–13.5、13.7 完成本机验证。13.6 的正目标候选准备阶段和关系请求级读连接复用仍未完成。Linux/Windows 原生写与已安装移动宿主未验收；本轮无提交、推送或发布。

## D34原生验收续记，2026-10-04

实际历史命名空间资格在 `407125f62fda994826a7858737b22fa95efe4cb4` 的[原生CI中22/22通过](https://github.com/loong10k/diskgraph/actions/runs/37161135994)，20项已审源码摘要均与该提交一致。两个Windows Rust版本和macOS Intel实际执行Engine11/FFI6，Linux ARM另执行2项真实原始字节文件系统用例。[D34回执](benchmarks/historical_namespace_acceptance_2026_10_04.json)保留准确用例、原始日志及此前失败观察；原有授权、响应预算与终态检查保留，CLI/MCP危险写工具仍关闭。

Q-04任务3.9仍缺完整设置、尺寸口径与provider兼容矩阵。Git库层配置/filter隔离、unborn/失败语义和共享探针预算已按15.13b/c/d及D29完成原生验收；任务8.7/15.13仍缺已授权采集入口、持久化及实际revision发布验收。全平台生产就绪父项保持未完成。
