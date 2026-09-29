# DiskGraph v1 行为基线

> P0 任务 1.1（RE-01）。记录日期：2026-09-28；源码基线：`main@10149ad`；
> 工具链：rustc 1.98.1（workspace 要求 ≥1.97），macOS arm64。
> 本文档固化 v1 已观测行为，作为 v2 演进与迁移的回归对照。**失败项：无。**

## 门禁实测结果（本机，`--locked`）

| 门禁 | 结果 |
| --- | --- |
| `cargo test --workspace --locked` | **54 通过 / 0 失败**（core 34、testkit 11、disktree 5、store 3、ffi 1） |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |

CI 配置为 macOS/Windows/Linux 三平台矩阵；本基线只声称 macOS 实测，其余平台以
CI 运行结果为准，不在此宣称。

## v1 五查询行为（`diskgraph-core/src/query.rs`）

- `top(parent, limit)`：按 `subtree_bytes` 降序、名称升序稳定 tie-break；不返回删除计划。
- `children(parent, offset, limit)`：确定性分页，`next_offset` 仅在还有剩余时给出。
- `explain(node_id)`：只返回已记录证据；无证据时返回空证据列表，不生成断言。
- `growth(before, after, locator)`：要求同 root、同非空 volume、**扫描设置完全一致**、
  时间单调、两侧 coverage 均 complete；重命名与不可达路径**故意不推断**，不匹配返回 None。
- `candidates(target_bytes)`：要求 coverage complete 且 target>0；仅接受显式
  `Rebuildable` 证据（confidence>0）的目录；受保护或被进程占用的祖先/后代阻断；
  候选互不重叠、按字节降序贪心累计到目标即停。**这是审阅队列，不是删除许可。**

## SQLite v1 存储行为（`diskgraph-store/src/lib.rs`）

- schema `user_version=1`：`snapshots`/`nodes`/`evidence` 三表，外键级联，写时校验。
- `save()` 是不可变事务插入；重复 snapshot ID 失败；部分无效图不落库（先 validate 再 BEGIN IMMEDIATE）。
- 校验规则：节点 ID 唯一、父节点必须存在、恰好一个根且 locator 与快照一致、
  locator 唯一、coverage 一致性（complete 时不允许 unreadable 或 depth-limited）、
  证据必须指向存在的节点且 confidence ≤ 100。
- 查询下推 SQL：`children` 按 `(subtree_bytes DESC, name ASC, id ASC)` 排序分页；
  `node_by_locator` 按精确 locator 键查找。
- u64 → i64 溢出显式报 `IntegerOverflow`，不静默截断。
- 跨连接持久化、无效图拒绝持久化均有回归测试。

## UniFFI JSON v1 行为（`diskgraph-ffi/src/lib.rs`）

- 信封：`{"schema_version":1,"ok":true,"data":...}` / `{"schema_version":1,"ok":false,"error":"..."}`。
- `capabilities_json` 诚实声明：原生路径扫描按平台、`document_uri_scan:false`、
  `cleanup_execution:false`；不支持的平台 scan 显式报错而非返回空成功。
- `scan_native_json` 扫描后持久化并返回 snapshot_id/node_count/coverage。
- `children_json` 的 limit 有界（1–1000），超界报错；分页用 `limit+1` 探测 has_more。
- `growth_json` 的 `delta_bytes` 是十进制字符串（跨语言整型安全）。
- `candidates_json` 只返回审阅候选；真实扫描后 evidence 为空 → 候选恒为空。

## 扫描桥接行为（`diskgraph-disktree`，pin `158f9cc`）

由 `crates/diskgraph-testkit/tests/scan_fixtures.rs` 固化：

- 稀疏文件：`apparent_size=true` 报 apparent（1 MiB），`false` 报分配块（实测远小于 apparent）。
- 硬链接：`dedup_hardlinks=true` 只计一次；关闭时第二个链接重复计入。
- symlink 环：默认 `follow_links=false` 不展开、不挂起，symlink 节点记为 `Symlink`。
- 不可读子目录：`coverage.complete=false` 且 `unreadable_nodes≥1`；目录仍在树中，`read_error=true`。
- 隐藏文件默认收录；真实扫描后 evidence 恒空，`candidates` 恒空。
- `max_depth` 截断：不产出界外节点，coverage 标记 `depth_limited`。

## 已知缺口（v2 计划消除，v1 保持原样）

1. 路径经 `to_string_lossy()`：非 UTF-8 名称在 v1 展示串中损坏（macOS APFS 拒绝
   创建此类名称——EILSEQ，Linux 字节透明文件系统上会真实出现）；v2 `Locator`
   已定义无损原始字节编码（`crates/diskgraph-core/src/locator.rs`）。
2. `file_identity` 恒为 None；Windows `volume_id` 为 None → Windows 跨快照 growth 不可用。
3. `growth`/`candidates` 在内存整图上计算（`load()` 全量加载）；无预算/分页上限。
4. `evidence` 恒空、`subject` 为自由文本；无实体/来源/时效模型。
5. v1 查询语义在 v2 中必须保持兼容（D6）；本文件即兼容性对照的锚点。

## 更换上游 pin 的强制流程

`crates/diskgraph-disktree/tests/pinned_upstream.rs` 锁定 `158f9cc…`。升级步骤：
跑路径/权限/链接/占位/大小口径回归 → 重跑 `tests/scan_fixtures.rs` 全部断言 →
更新本文件与 `docs/dependency-inventory.md`。
