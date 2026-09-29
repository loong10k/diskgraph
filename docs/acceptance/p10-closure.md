# P10 收尾验收（任务 11.3 / 11.4 / 11.5 / 11.6 / 11.7 / 11.9 / 11.10）

日期：2026-09-29 · 主机：macOS（darwin 27.0.0，arm64，Rust 1.98.1）

## 11.3 迁移演练（ST-02）

实测（`cargo test -p diskgraph-store --lib`）：

- `v1_databases_migrate_in_place_keeping_existing_rows`：v1 库原地迁移且既有行保留。
- `migration_backup_is_written_and_open_failure_keeps_v1_intact`：迁移前备份落盘；升级失败时 v1 数据保持完整可打开。

升级失败恢复路径即"备份存在 → 迁移失败 → v1 仍可打开"，旧版本消费者兼容由 v1 契约测试与 pinned 上游回归（`pinned_upstream.rs`）共同约束。控制库 v1→v2→v3 迁移链另有版本推进守卫测试。

## 11.4 效率对比（RE-03）

`scripts/evaluate.sh` 实测：**4/4 正确性检查通过**（无重建证据时候选为空、自比较干净、索引完成、尺寸为观测值），耗时对比基线写入 `dist/evaluation/BASELINE.md`。规模化数字见 `capacity-baseline.md`（20k 索引 1.18s、top-100 查询 1.1ms、5,000 成员分组 50ms）。

## 11.5 传输/宿主/平台验收表（AI-04 / RE-04）

| 维度 | 已验收 | 记录缺口 |
| --- | --- | --- |
| 传输 ×3 | stdio（Codex、Claude Desktop 真宿主，`host-acceptance`）；streamable-http（Codex 真调用 + 10 项协议检查，`http-acceptance`）；legacy-sse（5 项一致性，`legacy-acceptance`） | 全部汇总于 `transport-matrix.md` |
| 宿主 ×2+ | Codex CLI、Claude Desktop | PruneX 壳与 AgentScope 宿主待对侧产品 |
| 桌面 OS | macOS 全链真实验收；Linux 包结构 + 冒烟（`linux-package`，真实 systemd 运行留档待 Linux 主机）；Windows 语义 deferred（`real_os_requirements`） | Windows 真机 |
| 移动 | 绑定层 Kotlin 编译+JNA 运行 | NDK/Gradle/Xcode/真机全缺（`mobile-platforms.md`） |
| 语言宿主 | Swift CLI 宿主与 Kotlin JVM 宿主均真实编译运行（`ffi-bindings.md`） | XCFramework/AAR 打包待完整 Xcode |

## 11.6 运维演练映射（RT-03/04/05）

| 演练 | 证据（具名测试/脚本） |
| --- | --- |
| scope 撤销 | `a_revoked_scope_yields_no_plan`、`revoked_scopes_stop_accepting_jobs` |
| 服务重启 | `server_identity_and_scope_survive_an_engine_restart` |
| 密钥/认证 | 资源服务器认证负向测试族（伪造/过期/受众不符拒绝，P4-5.3） |
| 控制库备份/恢复 | `migration_backup_is_written_and_open_failure_keeps_v1_intact` |
| 容量预警 | `capacity_watermarks_refuse_new_work_without_touching_existing_data` + `capacity_report` 逐区读数 |
| 审计导出 | 审计脱敏（P4-6.15）+ `redacted_log`/`ExportPolicy::MetadataOnly`（日志/导出无正文通道） |
| 诊断不越权 | doctor 诊断面（P5）只读发现/预览，破坏性修复不自动执行 |

## 11.7 私有交付物（RE-05）

- `scripts/package-private.sh` 产物 `dist/private-macos/`：bin/（`diskgraph` + `diskgraph-mcp`，release）、`SHA256SUMS`、`MANIFEST.txt`（版本/平台/时间戳/rustc/profile）、`DEPENDENCIES.md`（依赖与许可证清单，回归规则由 `pinned_upstream.rs` 强制）、`licenses/`、LICENSE。
- Linux 侧 `scripts/package-linux.sh`（主机构建 + systemd 示例 + 冒烟），真实验证留档见 `linux-package`。
- 可复现构建说明：`MANIFEST.txt` 记录工具链指纹；锁定 `Cargo.lock` + pinned disktree rev。
- **明确阻塞（规格允许）**：代码签名与公证未配置——包未经签名，安装到启用 Gatekeeper 强校验的环境会被拦截；该阻塞写入交付说明，不以未签名包冒充分发物。

## 11.9 规格验证与差距审查

- 结构验证：14 个 spec 均含 Purpose / ADDED Requirements / Scenario 三段式；`Requirement:` 计数 77 与任务清单的规格引用一致（`requirements-matrix.md` 分母核对）。
- 差距审查结论（实现 vs 规格）：所有 deferred 项集中在 real-OS/跨产品边界（PF-04/05、8.9/8.10、9.3-9.6/9.8/9.9）与 CLI→ops 接线（CMD-01/03/04 的变更类命令）；这些在矩阵中为 partial/deferred 并留档，未混入已支持清单。
- `openspec` CLI 未安装于本机；结构验证以脚本化计数 + 本审查替代，安装 CLI 后可跑 `openspec validate --strict` 复核。

## 11.10 CI 门禁

`.github/workflows/ci.yml`：三 OS（macOS/Linux/Windows）矩阵执行 fmt + clippy(-D warnings) + 全量测试；macOS 腿额外做 UniFFI 绑定生成。隔离约定：real-host/破坏性演练一律 `#[ignore]`（`specialist_real_host`、`capacity_baseline`），常规 `cargo test` 不会触碰真实 cargo build/Docker/文件系统之外的主机状态——常规与隔离破坏性测试的分离由该机制保证，并在本仓库验证纪律中固化为"ignored 测试显式触发"。
