# 最新快照标识窄读验收

旧 `latest_native_snapshot_json` 已保留导出签名及 `schema_version=1 / ok / data.snapshot_id`，内部改走 `Engine::with_latest_snapshot_id_until`。原入口读取所有主体授权、全部范围清单及完整 snapshot JSON，且编码后没有末检；控制锁等待、编码后撤权及无用 JSON 解码三个真实回归均先失败，修复后通过。

现在按无损注册根窄读 scope ID，按实际 server/scope 读取有限 revision/snapshot ID，在拥有必要字段前累计字节预算；不读取范围显示文档或完整快照。非空及空结果都在首次授权前捕获撤权见证，编码后检查请求能力、实时数据库权限、原 scope/root/server 和已选择 revision 的实际归属。归属使用新鲜独立连接，末检不缓存允许结果或消费者 WAL 快照。未知原生通知能力时，授权代次变化仍返回 Conflict，不能释放结果，也不宣称精确撤权通知可用。原期限及至多 50ms 归属窗口不扩大。

本机 macOS ARM：最终目标 7/0/0；原 reply deadline 4/0、growth deadline 4/0、FFI 来源规范 8/0、Engine 来源规范 6/0通过。相关 Engine/Store/FFI all-targets 严格 Clippy、项目指定的排除 vendor 格式检查及 OpenSpec strict 通过。超大 ID 夹具通过当前正式发布 API 建立，额外确认 `ByteLimit`，不是用调度到期代替字节拒绝。完整原始压缩日志与来源摘要见 `docs/benchmarks/native_latest_authorized_narrow_read_2026_10_09/receipt.json`。

最初超大 ID 夹具直接 SQL 被外键和当前 writer 保护拒绝，改用正式发布接口；相关失败及一次引用参数编译错误均保留，不计作生产缺陷红灯。最初再授测试未检查原生 watch，实际返回正确的 Conflict 拒绝；修正为沿用现有 Engine 的未知通知语义，原失败记录保留。

旧库兼容原生测试在调用新查询前由扫描夹具返回 `Business(Unsupported)`，未取得兼容验收通过；Linux、Windows 的本增量、完整 workspace、真实旧库绑定与 release 性能仍待验证。原生扫描、保护安装及产品恢复监督父门禁不勾选，不归档变更，不声明生产就绪。
