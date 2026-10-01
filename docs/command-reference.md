# DiskGraph 命令与 MCP 接口参考

> 状态（2026-09-29，对照 `diskgraph --help` 与 MCP 传输实测）：`diskgraph` CLI 已实现 25 个根命令/族；MCP 服务器提供 stdio / streamable-http / legacy-sse 三种传输。查询类命令（scope/index/sync/status/snapshots/node/children/explain/related/top/growth/changes/search/explore/impact/candidates）全部可用；**变更类命令（duplicates/read/move/copy/trash/restore/purge/plan/apply）的 CLI 入口已存在但标注 not enabled in this build——其引擎与 ops 底层已实现并有测试（engine::content、diskgraph-ops），CLI→ops 的接线与产品化门禁尚未交付**；接线前它们不会执行任何文件操作。
> 正式依据：[command-surface](../openspec/changes/implement-diskgraph-platform/specs/command-surface/spec.md)；逐条验收状态见 [要求矩阵](acceptance/requirements-matrix.md)。

## 1. 接口约定

一个 `diskgraph` 可执行文件提供 29 个根命令或命令族。MCP 名称是拟定的稳定业务入口；同族以显式 action 参数区分，不开放任意命令字符串。CLI/MCP/FFI 调用同一 Rust 服务。

- 所有引用包含 server/scope/revision 语义；ID 不授予权限。CLI scope 别名仅在指定服务内解析。
- 本机为默认目标；`--server <configured-server>` 选择已配置、已验证身份的远程服务。远程绝对路径不映射本地文件。
- 普通查询只读已发布索引；未索引返回 not_indexed，不自动全盘扫描。
- 读取正文、索引管理、文件操作均不是“只读元数据”权限的附带能力。
- 文件动作默认只创建计划。只有 apply 在有效批准和实时重验后可能执行。
- 文档中的权限名称是目标应用权限，不等于某个 OAuth scope 字符串；实施时映射并测试。

权限缩写：M=`metadata:read`，C=`content:read`，I=`index:write`，S=`scope:admin`，F(action)=对应 `files:action`，O=授权范围内的操作查看/取消权限。API 返回实际可用能力，而不是看到命令就认为有权调用。

## 2. C01–C29 完整目录

| 编号 | CLI 命令/主要子动作 | MCP 服务工具 | 权限与效果 | 首交付阶段 |
| --- | --- | --- | --- | --- |
| C01 | scope add/list/show/remove | diskgraph_scope | list/show 按授权；add/remove 需 S | P1 |
| C02 | index --scope ID | diskgraph_index | I；创建持久扫描 job | P1 |
| C03 | sync --scope ID | diskgraph_sync | I；更新已注册范围，合并冲突作业 | P1 |
| C04 | status [--job ID] | diskgraph_status | M/作业所有权；版本、覆盖、能力与任务状态 | P2 |
| C05 | snapshots list/show/pin/remove | diskgraph_snapshots | 查看需 M；保留/删除索引需 I | P1 |
| C06 | changes BEFORE AFTER | diskgraph_changes | M；变化和不可比较原因 | P2 |
| C07 | growth NODE --since SNAPSHOT | diskgraph_growth | M；明确当前 revision 的历史增长 | P2 |
| C08 | explore --node ID 或 --query TEXT | diskgraph_explore | M；有界目录/关系摘要 | P2 |
| C09 | search --pattern TEXT | diskgraph_search | M；名称/路径/实体模式检索 | P2 |
| C10 | node NODE | diskgraph_node | M；单节点事实、口径、覆盖 | P2 |
| C11 | children NODE | diskgraph_children | M；直接子项、过滤、分页 | P2 |
| C12 | top --root NODE | diskgraph_top | M；范围内大项，口径明确 | P2 |
| C13 | related ENTITY --relation TYPE | diskgraph_related | M；方向、类型、深度受限 | P2 |
| C14 | explain ENTITY | diskgraph_explain | M；证据、时效、冲突、未知 | P2 |
| C15 | impact ENTITY | diskgraph_impact | M；已知可能影响，不授予执行权 | P2 |
| C16 | candidates --target-bytes N | diskgraph_candidates | M；待审阅/阻断/未知及目标缺口 | P2 |
| C17 | duplicates list/verify | diskgraph_duplicates | 疑似需 M；内容核验另需 C | P7 |
| C18 | read NODE --offset N --max-bytes N | diskgraph_read | C；有界普通文件内容，不修改文件 | P7 |
| C19 | move NODE --to DIRECTORY | diskgraph_move | M + F(move)；仅生成计划 | P5；跨卷 P6 |
| C20 | copy NODE --to DIRECTORY | diskgraph_copy | M + F(copy)；仅生成计划 | P5；跨卷 P6 |
| C21 | trash NODE | diskgraph_trash | M + F(trash)；仅生成回收计划 | P5 |
| C22 | restore RECOVERY --to DIRECTORY | diskgraph_restore | O + F(restore)；生成恢复计划 | P5 |
| C23 | purge RESOURCE-OR-RECOVERY | diskgraph_purge | 对应读取权 + F(purge)；不可逆计划 | P6 |
| C24 | plan show/validate/create | diskgraph_plan | 受限计划查看/验证；create 需对应动作权 | P5；专业计划 P6 |
| C25 | apply PLAN --approval REF --idempotency-key KEY | diskgraph_apply | 对应动作权 + 有效可信批准；可执行 | P5 |
| C26 | operations list/show/cancel | diskgraph_operations | O；持久执行状态和停止后续步骤 | P5 |
| C27 | serve --transport TYPE | 不作为 MCP 工具 | 本机启动服务、绑定配置的能力和范围 | stdio P3；网络 P4 |
| C28 | install add/remove/show --client ID | 不作为 MCP 工具 | 本机配置已有二进制的宿主注册 | P3 |
| C29 | doctor | diskgraph_doctor | 默认只读；仅显示主体可见诊断 | P3 |

阶段列是业务能力首次交付；MCP 映射在 P3/P4 起接入，文件工具随 P5/P6 启用。命令未实现、平台不支持或权限不足分别报告，不能用空成功输出凑齐数量。

`ls → children`、`tree → 有界 children/explore`、`rename → 同目录 move` 可以提供语法别名，但不计新增能力，也不要求安装系统同名命令。目录容量来自已有索引，不再启动 du 重扫。

## 3. 公共参数与结果

| 参数 | 适用范围 | 约束 |
| --- | --- | --- |
| --server | 支持远程的业务命令 | 配置名称/受控目标，不把 URI 当本地路径 |
| --scope | 业务范围 | 明确 ID 或服务内唯一别名；不得隐式扩大到全盘 |
| --revision | 版本化图查询/计划来源 | 缺省可选择 latest，但响应必须返回实际绑定版本 |
| --json | CLI | stdout 只有 JSON，日志/进度写 stderr |
| --limit / --cursor | 分页查询 | 游标绑定版本、过滤与授权，不能跨范围复用 |
| --depth / --max-nodes / --max-edges | 图遍历 | 服务端上限优先，不能设成无限 |
| --max-bytes / --timeout | 适用查询/内容/调用 | 输出与时间有上限；调用超时不等于作业终止 |
| --direction / --relation | related/impact | 只接受注册关系和允许方向 |
| --size-kind | 尺寸查询 | apparent/allocated；未知口径不自动替换 |

v2 envelope 包含 api_version、server/scope/revision、evaluated_at、coverage、data、warnings、truncated 和 next_cursor。字节/大整数用十进制字符串，未知为 null。ID 不解析为用户可自行拼接的文件路径。schema 样例见 [技术方案](technical-design.md)。

`--query` 是有界名称/模式定位提示，服务端不调用模型猜意图；歧义返回候选。MCP 宿主可先将自然语言转换为明确节点/过滤器。

长作业返回 job_id；扫描/哈希状态通过 status 查询，文件执行通过 operations 查询。连接取消不隐式回滚。内容 read 的 offset 指字节范围，返回实际范围、编码和截断；不能先把整文件读入内存再截断。

## 4. 范围与索引管理

```text
diskgraph scope add --root /path/to/project --name project
diskgraph index --scope project --json
diskgraph status --scope project --json
diskgraph snapshots list --scope project --limit 20 --json
diskgraph sync --scope project --json
```

这些仅展示拟定语法。scope add 需管理员允许的根范围、卷/provider 校验；远程管理工具不能任意注册未授权的系统根。只读客户端不能靠 index 参数扩权。

scope remove 只撤销注册/访问，不删除扫描对象，也不删除恢复记录。snapshots remove 只操作经保留策略允许的图历史，不删除用户文件或服务控制数据；引用冲突时明确拒绝或报告依赖。

普通查询命中已有索引但证据过期时返回 needs_sync/stale。对未索引 scope，用户需显式发起 index。不能为每个智能体进程重复全量扫描。

## 5. 精确查询与内容

```text
diskgraph explore --scope project --node NODE --revision REV --json
diskgraph children NODE --scope project --revision REV --limit 50 --json
diskgraph related ENTITY --scope project --relation owned_by_project --direction outgoing --json
diskgraph explain NODE --scope project --revision REV --json
diskgraph changes SNAPSHOT_BEFORE SNAPSHOT_AFTER --scope project --json
diskgraph compare --from-scope release --to-scope worktree --json
diskgraph compare --from REV_A --to REV_B --only-differences --limit 40
diskgraph compare --from-scope a --to-scope b --verify-content --verify-files 200 --json
diskgraph compare --from-scope release --to-scope worktree --plan --method mirror --json
diskgraph candidates --scope project --target-bytes 16106127360 --json
diskgraph duplicates list --scope project --json
diskgraph read NODE --scope project --offset 0 --max-bytes 4096 --json
```

`compare --verify-content` 需要先 `diskgraph grant --scope <id> --content-read`：内容读取是独立授权，不随 scope 注册而来。它只读取元数据已判为相同的行，并受 `--verify-files` 与 `--verify-bytes-per-file` 双重预算约束；未能核验的行计入 `verification.unverified`，summary 的 `is_complete` 相应为 false。`diskgraph grant --scope <id> --revoke-content-read` 收回该授权。

`compare --plan` 把同一份比较结果变成同步步骤，**不写、不移、不删任何文件，也没有能让它这么做的参数**。方向由 `--from`/`--to` 决定，不由方法名决定：`--from a --to b --method mirror` 的含义是"把 a 镜像到 b"，即让 b 完全变成 a 的样子——**删除 b 独有的文件**，把 a 独有的复制过去。三种方法：`update` 只复制（目标缺什么补什么、源更新就覆盖），`update-both` 双向更新较新的一侧（两侧同样新时复制两次，后者生效，因此不幂等），`mirror` 额外删除且 `deletes: true` 在 header 显式声明、删除步骤各自成条。无法读取大小（unknown-size）的行不产生动作，只进 `unresolved`。

内容哈希：核验通过的行带 `digests: {left, right}`，两侧哈希值都给出，可直接与外部清单比对。**用的是 SHA-256 而非 MD5**——抗碰撞更强，且 `digest_bounded` 已实现；`Evidence::Content` 表示该哈希确实读出，`Evidence::Metadata` 表示只比了大小和时间戳。

compare 回答的是"两棵树各自有什么"：根可以完全不同（发布产物对工作副本、两台机器、备份），这正是 changes 拒绝的情形——changes 问的是同一个目录随时间发生了什么。每一行给出 from-only / to-only / different / same（措辞与 `--from`/`--to` 参数对齐），different 再分 size / timestamp / contents / path / unknown-size。判定只到元数据层（大小、时间戳、秒级容差），**不读文件内容**，所以 same 意味着"在测到的深度上相同"，不是"逐字节相同"；内容级判定需要单独的 ContentRead 授权。无法读取大小的一侧报 unknown-size 而不是 different——那不是差异的证据。

candidates 的目标只是筛选目标，不是释放保证或自动清理指令。空候选、不足目标、未知占用覆盖都需要诚实返回，不能为了达到数字把风险规则放宽。

duplicates list 默认只给元数据疑似组；verify 必须明确组 ID、内容权限和预算，创建可取消核验 job。云占位默认跳过，不能触发下载；哈希不稳定则撤销确认。read 的内容不得通过 explain 或日志泄露给仅元数据主体。

## 6. 文件操作与专业清理

```text
diskgraph trash NODE --scope project --revision REV --json
diskgraph move NODE --scope project --to DIRECTORY --name NEW_NAME --json
diskgraph plan show PLAN --scope project --json
diskgraph plan validate PLAN --scope project --json
diskgraph apply PLAN --scope project --approval APPROVAL --idempotency-key REQUEST --json
diskgraph operations show OPERATION --scope project --json
diskgraph restore RECOVERY --scope project --to DIRECTORY --json
```

上面的动作语法都属于目标接口，当前没有实现，也不在本次执行。

plan show 展示精确项目、覆盖、风险、源/目标、数量/字节、元数据保真与恢复能力；validate 仅重验，不能改变已批准计划。需改目标时创建新计划。restore 的 --to 可省略以请求原位置，但冲突必须失败或重新计划。

批准不由 agent 自己生成。CLI 中 --approval 是受信批准引用，不是自由文本；MCP 不提供“给自己批准”的工具。幂等键复用且计划一致时返回原 operation，冲突则报 idempotency_conflict。

专业清理通过 plan create 的类型化动作：

| 动作类型 | 输入 | 约束 |
| --- | --- | --- |
| cargo_clean | project_id、已解析构建范围、受支持选项 | 验证 target/共享目录/活跃构建；缺 Cargo 不强删 |
| docker_cleanup | 明确 daemon/context 和精确对象 ID 列表 | 区分缓存/镜像/容器/卷；重验使用状态，不扩大成全局 prune |

仅允许注册动作和结构化参数，无 `command` 任意 Shell 字段。某工具无法表达精确批准边界时返回 unsupported。具体工具版本/项目配置风险必须出现在计划中。

trash 默认可恢复，但同卷回收不保证释放空间；purge 永久删除要独立批准。operations cancel 停止未来步骤，不把已完成项抹掉。恢复仅对仍存在且有恢复资料的资源有效。

## 7. 工具 profile、服务与安装

| Profile | 能力 | 备注 |
| --- | --- | --- |
| read-minimal | 如 explore/status/explain/changes | 降低默认工具列表长度，可配置，不是能力上限 |
| read-full | 完整精确元数据查询 | read 正文和 duplicates verify 仍需独立 C 权限 |
| manage | scope/index/sync/快照管理 | 显式授权；不得隐式继承文件写权限 |
| write | 计划/执行/恢复 | 默认关闭，逐动作权限+可信批准；不包含自我批准 |
| diagnostics | doctor/status | 只能显示主体可见的诊断信息 |

隐藏工具不是授权。直接构造隐藏写调用、改传输、伪造 scope 或复用其他主体游标必须失败。read/write 工具注解仅供宿主展示，不能替代服务端校验。

```text
diskgraph serve --transport stdio --scope project
diskgraph serve --transport streamable-http --config SERVER_CONFIG
diskgraph serve --transport legacy-sse --config SERVER_CONFIG
diskgraph install add --client CLIENT --scope project --dry-run
diskgraph install show --client CLIENT
diskgraph install remove --client CLIENT --dry-run
diskgraph doctor --json
```

TYPE 取 stdio、streamable-http、legacy-sse。旧 `serve --mcp` 示例可作为 stdio 兼容别名，不另计命令。HTTP 监听、TLS/隧道、认证、Origin 和配额由受控配置明确声明；默认 loopback，legacy-sse 默认关闭。

安装默认先提供可审阅差异；实际写配置使用显式 --apply-config，并校验目标客户端和并发修改。install remove 只移除 DiskGraph 管理的注册项，不卸载客户端、不删除索引或用户数据。卸载二进制与删除数据另行明确，不由 remove 猜测执行。

serve/install 不作为远程 MCP 工具；不能让服务器改本地智能体宿主配置。doctor 默认只读，不自动提权、装依赖、清理文件或强制解锁。

## 8. 错误、退出码与完成判断

以下是目标业务错误族；实现时固定并建立跨入口契约测试：

| CLI 退出码 | 业务结果 | 语义 |
| --- | --- | --- |
| 0 | ok / empty / job accepted | 成功返回；接受作业不代表作业已完成 |
| 2 | invalid_argument / ambiguous | 参数错误或需要限定对象 |
| 3 | permission_denied / approval_required | 权限或有效批准不足 |
| 4 | not_indexed / not_found | 未建索引或对象不存在 |
| 5 | stale_plan / revision_expired / incompatible_history | 需要重建计划、版本或比较条件不成立 |
| 6 | unsupported / unavailable | 平台/功能不支持或可选依赖不可用 |
| 7 | budget_exceeded / timeout / resource_exhausted | 预算、时间、容量限制 |
| 8 | partial / needs_attention | 工作未完整成功或需要核对 |
| 9 | conflict / idempotency_conflict | 对象/并发/幂等冲突 |
| 10 | internal_error | 不可恢复的服务故障，输出脱敏诊断 ID |

有界查询正常截断可返回 ok+truncated，不与业务 partial 混淆。MCP 使用结构化业务结果及适当工具错误标志，协议格式错误和业务拒绝分开；HTTP 网络状态也不代替业务结果。

验收必须覆盖每个命令的成功、无匹配、拒绝、过期、部分覆盖和不支持场景；命令存在、HTTP 200、返回 job ID、配置写入、绑定生成都不能单独算完成。
