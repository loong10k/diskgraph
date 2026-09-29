# P6 专业清理适配器真实主机验收（任务 7.11）

日期：2026-09-29 · 主机：macOS（darwin 27.0.0，arm64）· 执行方式：`cargo test -p diskgraph-ops --test specialist_real_host -- --ignored --test-threads=1 --nocapture`（测试内无模拟对象，全部命令走 `SandboxedRunner` 真实子进程）

## 结果：3/3 通过，演练对象零残留

| 演练 | 实测内容 | 结果 |
| --- | --- | --- |
| `the_cargo_adapter_reads_a_real_built_project` | 隔离目录真实 `cargo build`（hello-world，仅 std 依赖），inventory 读回 | 通过：cargo 1.98.1；target 实测 **1,040,264 逻辑字节**；`active_build=false`（锁未被持有） |
| `the_docker_adapter_creates_reads_and_removes_one_exact_volume` | 真实创建 drill volume → `docker_inventory` 读回 → `usage_check` 通过 → `docker_cleanup` 精确删除 → `docker volume inspect` 确认消失 | 通过：Docker 29.8.1；inventory 读回全部 15 个对象；清理后 inspect 退出码非 0；全仓 drill volume 残留 **0** |
| `the_docker_usage_check_refuses_a_volume_in_real_use` | 真实创建 volume + 停止容器挂载（`docker create --mount source=…,target=/data alpine:3.20`）→ 使用检查 | 通过：被停止容器引用的 volume 被拒（`InUse`），且事后 `volume inspect` 确认 volume 未被任何路径删除 |

同时验证的负向事实（源自单元测试，路径 `crates/diskgraph-ops/src/docker.rs` / `specialist.rs`）：

- 清理 argv 中**永不出现 `prune` 或通配符**；删除命令只携带被批准对象的精确 id。
- Docker 仍认识对象且输出无法证明删除时 → `NeedsAttention`，不宣称成功（7.10）。
- shell 注入载荷（`$(whoami)`、反引号、管道）作为**单个 argv 字面量**透传，无 shell 参与（7.6）。
- 超时杀子进程、输出截断置位、瞬时失败按重试预算重跑（7.6）。
- purge：默认执行器完全禁用；仅配置的授权面签发的批准可执行；对象被替换后身份不符拒绝且冒名文件无损；crash 后重试返回原操作不再删除（7.3/7.4）。

## 测量限制（必须与数字一起读）

1. **逻辑大小 ≠ 净释放证明。** 上表字节数是文件系统观察值（`stat` 聚合与 Docker 自报），不是回收保证。回收以操作前后卷空闲差值（`VolumeReport::free_delta_bytes`）为准，且必须附带并发写入者的存在说明。
2. **Docker VM 磁盘。** Docker Desktop 的镜像/卷字节位于 Linux VM 磁盘映像内：删除对象不会立即缩小宿主文件（`VM_CAVEAT` 随每次库存输出）。VM 磁盘映像本身永不作为普通缓存目录呈报。
3. **打开句柄/快照/并发写入。** 演练期间若有其他进程写入，空闲差值包含其影响；macOS APFS 快照与打开句柄会阻止块归还。本机演练在无并发写入的隔离目录进行，但该前提不外推到生产。
4. **活跃构建探测口径。** `.cargo-lock` 文件在构建结束后常驻（本机实测证据：`target/debug/.cargo-lock` 存在于空闲工作树），故存在性不构成活跃证据；探测以**非阻塞 flock 争用**为准，单元测试以真实持锁验证双向。
5. **未覆盖项。** purge 的真实授权面（PruneX 审阅 UI）尚不存在，验收使用测试内的 `ApprovalIssuer` 充当受信面；Linux 主机上的同等演练未在本机执行（见 `docs/acceptance/linux-package.md` 的主机限制说明）。
