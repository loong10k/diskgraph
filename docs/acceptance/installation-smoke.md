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
