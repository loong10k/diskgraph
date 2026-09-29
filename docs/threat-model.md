# DiskGraph 威胁模型与信任边界

> P0 任务 1.9（SC-05 / OP-03 / EC-04 / RE-02），依据 design.md D1–D15。
> 本文映射攻击路径 → 负向测试；"现有测试"列中的测试名是机器可验证的锚点，
> 任何重构不得使这些测试消失或弱化。标注 P5+ 的行在对应阶段实现时补齐测试。

## 1. 资产与信任边界

| 资产 | 说明 |
| --- | --- |
| 用户文件与目录 | 最终保护对象；任何误删不可逆风险都围绕它 |
| diskgraph.sqlite | 可重建图索引；泄露即暴露用户文件布局 |
| diskgraph-control.sqlite（计划中） | scope、批准、操作、恢复、审计；**不可随索引重建丢失** |
| 批准凭据 | 来自可信通道；被智能体伪造即突破 D10 |
| 服务身份（server_id/scope/principal） | 远程授权的地基（D3/D9） |
| 平台密钥/安全书签 | 由平台密钥库保管，库内只存引用 |

信任边界：智能体/模型 ↔ DiskGraph 服务 ↔ 可信审阅通道 ↔ 文件系统；
远程客户端 ↔ 网络传输 ↔ 服务器本地范围（D9）。模型输出**永远**只是建议：
不能改写观察事实、不能签发批准、不能扩大 scope。

## 2. 攻击路径 → 负向测试映射

| # | 攻击路径 | 防线（design） | 现有测试 | 状态 |
| --- | --- | --- | --- | --- |
| A1 | 伪造/拼接 `ResourceRef` 跨服务引用同名路径 | D3：四元组全匹配才算同一资源 | `ids::same_named_paths_across_servers_are_distinct_resources`、`revision_or_scope_change_makes_a_reference_incomparable` | ✅ |
| A2 | 非法 ID 注入（空、超长、特殊字符、伪装 node_id） | ID 契约校验 | `identifiers_reject_empty_overlong_and_special_characters`、`node_id_round_trips_as_decimal_string_and_rejects_other_shapes` | ✅ |
| A3 | 元数据授权被扩张为内容/写入 | SC-03：能力互不蕴含 | `metadata_grant_does_not_imply_content_index_or_file_actions`、`each_file_action_is_its_own_capability` | ✅ |
| A4 | 撤权/过期策略后继续用旧授权 | SC-04：策略版本 + 撤销 | `publishing_a_new_policy_version_expires_old_grants`、`revocation_denies_everyone_until_republish` | ✅ |
| A5 | 新装服务被诱导执行修改（OP-01） | 默认 DenyAll | `default_authorizer_denies_everything_including_reads` | ✅ |
| A6 | 无授权主体跨 scope 访问 | SC-02：scope 绑定 grant | `grants_do_not_cross_scopes_or_principals` | ✅ |
| A7 | 仅元数据 provider 被用于读正文/删文件 | 能力探测 → unsupported | `metadata_only_provider_reports_unsupported_for_content_and_mutations` | ✅ |
| A8 | 无效图注入脏数据（假父节点、假证据、重复 ID） | store 前置校验 | `rejects_invalid_graph_without_persisting_a_partial_snapshot`（store） | ✅ |
| A9 | 分类提示被当成删除许可 | candidates 只认显式 Rebuildable 证据 | `candidates_never_include_a_protected_descendant`、`scans_hidden_files_without_granting_delete_authority` | ✅ |
| A10 | 部分扫描冒充完整（遮挡误删判断） | coverage 一致性 + growth 拒绝 partial | `queries_are_bounded_and_growth_requires_complete_compatible_scans`、`unreadable_directory_forces_incomplete_coverage` | ✅ |
| A11 | locator 损坏/伪造展示串定位文件 | 展示串非操作目标；raw 字节解码失败显式报错 | `corrupt_base64_is_reported_not_loaned_as_a_display_string`、`non_utf8_names_survive_v2_round_trip_but_not_v1_strings` | ✅ |
| A12 | 上游 pin 被悄悄替换/引入 disktree-app/PruneX 依赖 | pin 回归测试 | `disktree_core_is_pinned_to_the_reviewed_revision`、`no_upstream_app_or_product_dependency_leaks_into_the_workspace` | ✅ |
| A13 | 生成绑定/vendor 源码入库 | 仓库边界测试 | `generated_bindings_stay_out_of_the_repository` | ✅ |
| A14 | 智能体自签批准（approved=true、自由文本、--yes） | OP-03：批准只来自可信通道；MCP 无自批工具 | 契约已定（无 Approve 权限变体）；签名/绑定/过期测试在 P5（任务 6.2） | 🅿5 |
| A15 | 计划批准后目标被替换（TOCTOU、symlink 竞态） | OP-04：执行前实时重验 | P5（任务 6.7/6.8） | 🅿5 |
| A16 | 幂等键重放导致重复执行 | OP-08 | P5（任务 6.4/6.13） | 🅿5 |
| A17 | 回收失败退化为永久删除 | OP-06 | P5（任务 6.10） | 🅿5 |
| A18 | 子进程参数注入（cargo alias、Git hooks、Docker context） | EC-04：固定 argv/环境/预算 | P6（任务 7.6） | 🅿6 |
| A19 | 远程代理伪造身份头 / Origin / 令牌泄露 | D9 + MCP-06 | P4（任务 5.3/5.4/5.9） | 🅿4 |
| A20 | 云占位在扫描/读取时被静默下载 | FS-05 / CT-02 | 需真实 provider（`testkit::real_os_requirements` 清单） | 🅿真机 |
| A21 | 控制库被当缓存删除导致恢复记录丢失 | D5：图库可重建、控制库必须备份 | 恢复/对账测试在 P5（任务 6.15） | 🅿5 |

## 3. 不可信数据处理

文件名、清单内容、证据文本、Git 输出、Docker 输出都是**不可信数据**：
- 不得成为工具指令或 SQL/Shell 片段（当前唯一子进程面为 P6 适配器，届时固定 argv）。
- 展示路径与操作定位分离（`Locator.display` 仅日志/UI）。
- 名称/模式匹配只进查询提示，不参与身份判定（FTS5 不参与身份）。

## 4. 与规格的追溯

每个 A 编号在最终验收（任务 11.2 写能力安全总门禁）时必须全部为 ✅。
`real_os_requirements()`（testkit）是 A20/A21 类"必须真机"场景的机器可查清单；
阶段推进时在此表同步新增行并保持"行 → 测试名"一一对应。
