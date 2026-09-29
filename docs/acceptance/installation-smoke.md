# 安装冒烟验收（CLI 安装 + Codex 模型驱动闭环）

日期：2026-09-29 · 方式：真实安装到本机默认路径，再用真实智能体会话驱动，不用测试桩

## 安装

| 产物 | 安装方式 | 结果 |
| --- | --- | --- |
| `diskgraph` CLI | `cargo install --path crates/diskgraph-cli --locked` → `~/.cargo/bin/diskgraph` | ✅ `diskgraph --version` = 0.1.0 |
| `diskgraph-mcp` 服务 | `cargo build --release -p diskgraph-mcp` | ✅ 3.9 MB，stdio 默认 |
| Codex 注册 | `codex mcp add diskgraph -- <release 二进制> --data-dir … --profile all` | ✅ `codex mcp get diskgraph` 报 enabled: true / stdio |

## CLI 全链（12 项真实调用，demo 目录 6 文件三子树）

scope add → index --wait（completed + revision 发布）→ node（coverage complete，8 files/7 dirs，subtree 1,175,552B）→ top（按占用排序：myproject 794,624 > data 245,760 > archive 135,168）→ search --pattern（命中 app.bin）→ 重复 index（复用不重复发布）→ explain project-7（实体 + 1 条证据边）→ related（typed relation `owned_by_project`，evidence refs `supports`）→ candidates（空 + review_only=true）。

参数教训（真实口径）：scope 只以完整 `scope-…` ID 寻址（无名字别名）；explain/related 需要 `--revision` 且 entity 是字符串 ID（`project-7`）而非节点号。

## Codex 模型驱动闭环

`codex exec` 会话中模型自主发现 diskgraph 工具并完成任务：`diskgraph_scope` 返回 `permission_denied`（MCP 主体无 scope:admin——**默认拒绝语义在真实宿主中如实生效**），模型转用 `diskgraph_status` 报出：scope 路径、**8 files**、最大子目录 **myproject = 794,624 字节**——与 CLI 直连同一数据库的结果逐字一致（同源交叉验证）。

## 结论

安装 → 注册 → CLI 全链 → 模型驱动调用，四层全部真实通过；未通过的调用（scope 管理）是授权模型按设计拒绝，不是缺陷。

## 追加：真实主目录全量扫描（2026-09-29）

对 `/Users/wandl`（453 万真实节点）完成全量索引发布：4 分 17 秒，coverage 完整，revision `rev-c3bffcb9`。disktree 风格 JSON tree 现由 CLI 一等命令渲染：`diskgraph tree --scope ID [--revision REV] --depth N [--min-bytes N]`——core `render_tree` 走正式查询层（metadata 授权 + 信封），depth 截断与 min_bytes 过滤均诚实标注（`truncated`/`children_count`/`hidden_below_min_bytes`）。首版曾用临时 Python 脚本直读 SQLite 渲染，`diskgraph tree` 落地后与之逐字节对账一致（430 万节点树，Rust 23.8s vs Python 39.6s），脚本随即退役：深度 3 全树 454 KB / 39.6 秒，可交互钻取（home → workspaces 119 GB → workspace-partme-ai → diskgraph 仓 target 3.4 GB）。

**验证过程抓到并修复的真实缺陷**（回归测试 `byte_charges_bill_each_file_once_not_once_per_ancestor` 锁定）：

1. **预算计费随深度放大**：`execute_scan` 按 `subtree_bytes` 逐节点计费，每个文件的字节被所有祖先重复计费——深树把 395 GB 内容虚增为数 TB，触发 `StagingLimit` 停止。修复为按节点自身 `direct_bytes` 计费。
2. **CLI 预算参数不可配置**：新增全局 `--max-nodes-per-scan` 与 `--max-staging-bytes`，同一参数同步驱动硬拒绝上限与逐节点计费预算（此前只动硬上限会被计费预算拦住）。
3. **预算停止无诊断输出**：`budget_stop` 分支现在向 stderr 打印具名原因与计费水位（`scan stopped: StagingLimit after N nodes / M bytes / T ms`），失败作业无需调试器即可归因。

门禁：325 测试全绿，fmt/clippy 干净。
