# 要求证据矩阵（P10 任务 11.1 / 11.2）

2026-10-02 更新：当前 change 有 **90 个唯一 Requirement ID / 92 个声明**；OP-10、ST-05 在不同增量段重复声明，分母不能简单按标题行数计。以下 77 条是原始基线，补充 13 条列于末尾。`supported` 只表示具名实现/回归的限定范围，不能替代完整平台门禁；全平台状态见[当前实施记录](../production-readiness-full-platform-2026-10-02.zh-CN.md)。

日期：2026-09-29 · 分母核对：14 个 spec、**77 条 Requirement**（`grep -c "Requirement:"` = 77，逐条列出于下）。状态口径：**supported** = 实现且具名测试通过；**partial** = 实现但有记录在案的边界；**deferred** = 需要 real-OS/另一产品侧，登记于 `diskgraph_testkit::real_os_requirements()` 与各验收文档，不冒充完成。测试名均为仓库内真实测试（分母 326 个唯一测试名，未编造）。

## agent-integration

| ID | 状态 | 证据 |
| --- | --- | --- |
| AI-01 | supported | engine/store 无 PruneX 依赖；`the_ffi_layer_is_independent_of_any_host_application`（ffi）；P1-2.2 独立启动测试 |
| AI-02 | supported | scope 撤销可逆（`revoked_scopes_stop_accepting_jobs`）；配置只写引擎自身 data_dir |
| AI-03 | supported | 同 scope 作业合并（`per_principal_job_quotas_refuse_excess_without_running` + 2.8 合并测试）；扫描从不隐式触发（显式 index_scope） |
| AI-04 | partial | stdio（Codex/Claude Desktop）与 streamable-http（Codex）真宿主验收见 `host-acceptance`/`http-acceptance`；legacy 宿主为协议一致性验收（`legacy-acceptance`） |

## bounded-queries

| ID | 状态 | 证据 |
| --- | --- | --- |
| Q-01 | supported | `queries_are_bounded_and_growth_requires_complete_compatible_scans`；CLI/MCP/FFI 同 core 函数路径 |
| Q-02 | supported | `capacity-baseline.md`（20k 文件、top-100 1.1ms）+ 页界 API |
| Q-03 | supported | `queries.rs` explore/impact 方向化传播 + 保护不传播测试 |
| Q-04 | supported | `growth` 要求完整/同设置/同卷快照；`snapshot_json_round_trip_preserves_locators` |
| Q-05 | supported | `candidates_never_include_a_protected_descendant`（候选=审查队列非授权） |
| Q-06 | supported | 新鲜度契约（freshness 层 + P4 游标纪元测试） |
| Q-07 | supported | JSON envelope 统一（schema_version/ok/data/error）；FFI 往返测试 |

## command-surface

| ID | 状态 | 证据 |
| --- | --- | --- |
| CMD-01 | partial | CLI 命令族已实现（P2 14 命令）；11.8 文档逐命令核对进行中 |
| CMD-02 | supported | 共享错误码表（P0 契约）+ envelope |
| CMD-03 | supported | 变更类命令走计划（ops `PlanBuilder`，OP-02 测试族） |
| CMD-04 | partial | MCP 工具与 CLI 同底层；全量映射清单待 11.8 复核 |
| CMD-05 | supported | `a_revoked_scope_yields_no_plan`；scope/别名不产生删除语义 |

## content-inspection

| ID | 状态 | 证据 |
| --- | --- | --- |
| CT-01 | supported | `content_read_requires_the_content_grant`、`a_read_stops_at_the_byte_budget_and_says_truncated`、`a_read_refuses_links_directories_and_out_of_scope_paths` |
| CT-02 | partial | macOS HydrationGuard 线程策略及恢复真实测试；占位 fake 与稳定性回归通过；Linux/Windows 原生不下载保护与真实云 provider 未验收 |
| CT-03 | supported | core duplicates 组（`equal_size_is_a_suspect_never_a_verdict`）+ `digest_bounded` 不稳定即作废 + `duplicate_suspects_group_by_size_and_mark_hard_links` |
| CT-04 | supported | `metadata_only_export_drops_bytes_and_names_their_count`、`the_redacted_log_never_carries_content`、`the_metadata_only_export_cannot_carry_body_bytes` |

## ecosystem-adapters

| ID | 状态 | 证据 |
| --- | --- | --- |
| EC-01 | supported | 无 shell 依赖的确定性 collector（`collectors.rs` 5 测试）；卷容量经 statfs 非 `df` |
| EC-02 | supported | `git_samples_dirty_stash_and_upstream_honestly`（离线、无 upstream≠已推送）；`the_usage_sample_sees_the_test_process_own_open_file` |
| EC-03 | supported | 适配器允许列表 + unavailable 语义（`an_unlisted_capability_is_refused_even_when_known` 等 12 测试）；Docker 无 prune（`cleanup_never_widenes_beyond_the_exact_objects_and_never_prunes`） |
| EC-04 | supported | `structured_arguments_pass_through_without_a_shell`、`a_timeout_kills_a_stuck_child`、`output_past_the_cap_is_discarded_and_flagged` |

## filesystem-snapshots

| ID | 状态 | 证据 |
| --- | --- | --- |
| FS-01 | supported | `ScanWindow` 起止/选项指纹（P1-2.5，`ScanWindow.duration_ms` 测试） |
| FS-02 | partial | 定位器原始字节/UTF-16 往返通过；pinned scanner 不可逆名称拒绝发布；Windows 属性句柄身份已实现，原生回归待 CI，不宣称已支持全部名称 |
| FS-03 | supported | 硬链接去重一次（`hardlinks_count_once`）；st_blocks≠可释放口径（P1-2.6） |
| FS-04 | supported | 排除与错误分离（`ScanExclusions`）；挂载替换 real-OS 留档 |
| FS-05 | partial | 不跟随链接/环（`symlink_loop`）；占位不下载（PlaceholderPolicy 默认降级）；真设备留档 |
| FS-06 | supported | `a_rescan_counts_a_missing_path_without_deleting_it`、`a_controlled_rescan_records_absence_without_deleting_history`；事件订阅轮询语义（`the_polling_watcher_reports_...`，平台事件绑定留档） |

## mcp-transports

| ID | 状态 | 证据 |
| --- | --- | --- |
| MCP-01 | supported | stdio + streamable-http + legacy-sse 三传输；`transport-matrix.md` |
| MCP-02 | supported | http_acceptance 10 项 + legacy 5 项协议一致性 |
| MCP-03 | supported | `origins_are_validated_before_anything_else`、非回环绑定需认证、客户端同名目录不被当作服务端目标（P4-5.1 验收） |
| MCP-04 | supported | 统一授权执行（engine require 贯穿三传输） |
| MCP-05 | supported | `the_server_stream_survives_read_timeouts`；连接身份与业务 job ID 分离（P4-5.8） |
| MCP-06 | supported | `HttpLimits`（body/连接/速率/并发）及有界 503 关闭；15.9 增量覆盖 4096 桶/64 字节键、单调补充、安全闲置回收、IPv6/可信链和重复 XFF socket 回归，三平台新 SHA CI 待核对；IP/单实例配额及满表取舍见 MCP README |

## platform-ffi

| ID | 状态 | 证据 |
| --- | --- | --- |
| PF-01 | partial | 绑定生成+Swift/Kotlin 真实宿主运行（`ffi-bindings.md`）；异步句柄+取消（ffi 4 测试）；AAR/XCFramework 打包 deferred（9.9） |
| PF-02 | partial | 引擎/控制双库独立生命周期（P1-2.2）；`the_ffi_layer_is_independent_of_any_host_application`；SwiftPM/GRDB 动态宿主真实并发 CRUD、FD 释放和重开；静态嵌入/Room deferred |
| PF-03 | partial | macOS 真实验收；Windows/Linux 矩阵 deferred（8.9/8.10 留档） |
| PF-04 | deferred | 无 NDK/设备（`mobile-platforms.md`） |
| PF-05 | deferred | 无 Xcode/设备（`mobile-platforms.md`） |

## relationship-evidence

| ID | 状态 | 证据 |
| --- | --- | --- |
| EV-01 | supported | 类型化实体/边 + 传播规则（P2-3.1 测试族） |
| EV-02 | supported | Git 采集不联网、upstream 缺失≠已推送（`git_samples_dirty_stash_and_upstream_honestly`） |
| EV-03 | supported | 轮询 watcher 全量 diff 无丢事件 + 溢出请求受控重扫；平台原生绑定留档 |
| EV-04 | supported | 确定性项目证据（collectors 5 测试，RULE_VERSION 化） |
| EV-05 | supported | revision 选择与过期明确报错（store `RevisionNotFound` + engine load_revision） |
| EV-06 | supported | 进程占用采样带 coverage（`a_missing_probe_is_unobservable_never_nobody`——不可观测≠无人使用） |

## release-evaluation

| ID | 状态 | 证据 |
| --- | --- | --- |
| RE-01 | supported | 本矩阵即逐条复查；未完成能力全部留档不冒充（PF-04/05、9.3-9.6/9.8/9.9、8.9/8.10） |
| RE-02 | supported | 安全总门禁见下节映射（伪造批准/重放/竞态/崩溃/满盘/取消/恢复冲突十类全具名）；real-OS 场景显式登记不模拟 |
| RE-03 | supported | `capacity-baseline.md` + `evaluate.sh`（正确性 4/4 + 时间对比） |
| RE-04 | partial | 三传输两宿主 macOS 验收齐；Linux 包结构验收（真实运行留档）；Windows/移动 deferred |
| RE-05 | partial | 打包脚本 + 校验和 + 许可清单（`delivery-11.7.md`）；签名/公证缺失明确阻塞（规格允许） |

## runtime-governance

| ID | 状态 | 证据 |
| --- | --- | --- |
| RT-01 | supported | 持久作业/租约/合并（P1-2.8 + `queued_jobs_cancel_without_running_and_terminal_jobs_never_rerun`） |
| RT-02 | supported | 预算具名停止（`budgets_stop_a_walk_for_a_named_reason`）+ 取消不推进 latest（`cancellation_is_a_named_stop_rather_than_a_silent_finish`） |
| RT-03 | supported | 控制库存活于图历史删除/重建（P4-6.15）；审计脱敏 |
| RT-04 | supported | `capacity_watermarks_refuse_new_work_without_touching_existing_data`、逐区计量（`capacity_report`） |
| RT-05 | supported | doctor 诊断（发现/撤销批准/空文件往返，P5 交付）；诊断不自动执行破坏性修复 |

## safe-file-operations

| ID | 状态 | 证据 |
| --- | --- | --- |
| OP-01 | supported | `purging_is_refused_rather_than_silently_enabled`、purge 默认无 authority 即禁用 |
| OP-02 | supported | `a_plan_is_immutable_and_digest_addressed`、`parent_and_child_requests_collapse_to_the_child` |
| OP-03 | supported | `an_approval_is_bound_to_the_plan_and_its_principal`、`an_expired_approval_does_not_verify`、purge 独立授权（`only_the_configured_authority_may_approve_a_purge`） |
| OP-04 | partial | `an_object_replaced_after_planning_is_refused`、链接植入拒绝（2 测试）、`a_purge_refuses_a_swapped_object_and_never_touches_the_imposter` |
| OP-05 | partial | 跨卷 staging 校验发布（`a_cross_volume_copy_stages_verifies_then_publishes`）、确认后删源（`a_cross_volume_move_publishes_before_the_source_is_removed`）、`an_occupied_target_is_never_overwritten` |
| OP-06 | partial | `trash_moves_into_quarantine_and_never_deletes`、`a_restore_never_overwrites_a_reoccupied_original`、无安全回收即 unsupported |
| OP-07 | supported | `restoring_after_a_purge_reports_irrecoverable`、purge 不可恢复语义 + 独立授权 |
| OP-08 | supported | `a_retried_key_returns_the_original_operation_without_moving_twice`、`a_parked_purge_retry_returns_the_same_operation_without_replaying` |
| OP-09 | supported | `cancelling_a_finished_operation_leaves_it_untouched`、部分完成 Partial 状态（批量测试） |
| OP-10 | supported | VolumeReport 三分列（处理/保留/空闲差）+ `refresh_scope_after_operation`；并发写入限制说明随报告 |

## scope-authorization

| ID | 状态 | 证据 |
| --- | --- | --- |
| SC-01 | supported | `a_revoked_scope_yields_no_plan`；移除 scope 不删文件（2.1） |
| SC-02 | supported | server 身份绑定资源（P1-2.2 + restart 测试） |
| SC-03 | supported | content:read 独立于 metadata（`content_read_requires_the_content_grant`） |
| SC-04 | supported | `revoking_an_approval_mid_flight_stops_the_next_step`、`operations_are_listed_and_shown_only_to_their_principal`、游标绑定 principal（P4-5.9） |
| SC-05 | supported | 资源服务器认证（伪造/过期/受众不符拒绝，P4-5.3 负向测试）；导出元数据优先（CT-04 复用） |

## snapshot-storage

| ID | 状态 | 证据 |
| --- | --- | --- |
| ST-01 | supported | staging/事务发布/latest 指针 + 崩溃不泄漏半发布（P1-2.4） |
| ST-02 | supported | v1→v3 版本化迁移测试（备份不足/半途失败/旧客户端拒绝） |
| ST-03 | supported | 稳定分页 + 失效游标报错（P2-2.10） |
| ST-04 | supported | 清理历史不破坏引用（pin/remove 依赖检查，P2 测试） |
| ST-05 | supported | 数据目录权限与独立所有权（P1-2.2） |

## 11.2 写能力安全总门禁十类映射

| 攻击类 | 具名测试 |
| --- | --- |
| 越权 | `unauthorized_principals_are_denied_everywhere`、`content_read_requires_the_content_grant` |
| 伪造批准 | `apply_without_a_valid_approval_moves_nothing`、`a_revoked_approval_blocks_execution`、purge authority 测试族 |
| 重放 | `a_retried_key_returns_the_original_operation_without_moving_twice`、`a_reused_key_with_a_different_approval_is_refused`、`a_plan_cannot_be_applied_twice` |
| 目录竞态（链接植入） | `a_link_planted_in_the_source_after_planning_stops_the_move`、`a_link_planted_in_the_destination_stops_the_move`、`an_object_replaced_after_planning_is_refused` |
| 挂载变化 | `MountReplacement` real-OS 登记（单元层 same-volume 拒绝 + 跨卷 staging 覆盖语义） |
| 崩溃 | `a_crash_after_intent_leaves_the_operation_needing_attention`、`a_crash_after_the_file_moved_is_never_replayed_blindly`、跨卷 seam 停靠测试 |
| 并发 | `refuse_conflicts` 路径 + `per_principal_job_quotas_refuse_excess_without_running` |
| 满盘 | `capacity_watermarks_refuse_new_work_without_touching_existing_data`、迁移备份不足测试 |
| 取消 | `queued_jobs_cancel_without_running_and_terminal_jobs_never_rerun`、`cancelling_a_finished_operation_leaves_it_untouched`、digest cancel void |
| 恢复冲突 | `a_restore_never_overwrites_a_reoccupied_original`、`a_restore_of_a_vanished_object_is_refused` |

## 2026-10-02 增量要求

| ID | 状态 | 证据与边界 |
| --- | --- | --- |
| Q-08 | partial | 有界有序历史、邻接 UNION ALL keyset、解码前实体/证据预算及 200k 无关关系 VM 工作量回归；TUI 单连接共同预算及最终授权；schema 9 精确宽目录聚合/known 页的 200k VM 工作量与 10 项聚合/迁移/旧 writer/目录续页回归通过；绑定 v2 children 和 search seek 深页 VM 工作量及页外 probe 不解码通过。显式 offset 仍有跳读成本，增量平台门禁继续验证 |
| Q-09 | supported | 正目标候选窄读、完整/截断/缺口报告、20k/200k 配对 release 基准 |
| OP-13 | partial | macOS 库内逐块实时授权/期限/批准/取消、源版本绑定和终态 CAS 回归；对应 OS 原生门禁仍未全部完成 |
| PF-06 | partial | 持久服务、关闭、撤权、真实进度、同根共享句柄/last-drop 与 job/fence revision 18 项回归；GUI/provider 调度及正式包未验收 |
| PF-07 | supported | MetadataRead/OperationView 分别在 running/completed 撤销，progress/poll/result 均拒绝；Engine stale authorizer 与 scope/admin fallback 回归 |
| SC-06 | supported | 请求主体/token 与实时授权交集，真实 socket、跨 scope revision、显式旧 revision/non-root node 回归 |
| CT-05 | supported | 总文件/字节/期限预算，失败读取真实成本、部分摘要不确认、取消/撤权回归 |
| RT-06 | supported | 20 ms 协作取消、实际编码 staging、容量入队拒绝、30 秒租约/5 秒续租与 fencing 夹具；非严格 RSS |
| RT-07 | supported | prune 默认预览，保护 latest/pin/操作与恢复引用；SQLite 不自动 VACUUM |
| RE-06 | partial | f57aa40 与存储结构 63bc6c7 的同 SHA 22/22 CI 已通过；后续纯文档 1ba56aa 为 21/22，Windows 连接上限发生 TCP reset，已先红后绿修复，23cfb52 的[同 SHA 22/22 CI](https://github.com/loong10k/diskgraph/actions/runs/36989443313)及本机 584 测试/Clippy/协议和独立复审通过。真实长期生产运行/签名发行仍缺；未部署生产 |
| RE-07 | partial | 本轮独立审查、真实 FFI、release 性能与桌面 CI 工作流；移动/provider/原生写/签名/生产证据尚缺，不能宣称全平台就绪 |
| RT-08 | supported | `dropping_an_idle_runner_releases_its_engine` 和 `dropping_a_runner_blocked_on_queue_read_prevents_a_new_claim` 先红后绿；f57aa40 跨平台通过；已开始扫描仍按预算/租约协作结束，不承诺瞬时取消 |
| ST-06 | supported | 入口 93 行、每类型独立文件、中文实际来源和参数/返回、无生产 wildcard/stub；`production_storage_entry_types_and_imports_follow_the_rust_contract` 先红后绿。store 76 passed / 5 ignored，workspace 582 / 13；两位独立复审确认旧 API/事务与 185 处 SQL 不变；[63bc6c7 的 22/22 跨平台 CI](https://github.com/loong10k/diskgraph/actions/runs/36986111434)通过 |
