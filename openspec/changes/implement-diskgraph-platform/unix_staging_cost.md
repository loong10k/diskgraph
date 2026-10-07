# Unix staging 实际编码预算

状态：实施中，沿用已确认实际编码预算，不改变容量或授权门禁。

每条旁表按实际 observation.encode() 或 gap.code() 的字节数，加 node_id 与 writer_generation 两个固定逻辑整数各8字节计量；不将版本值14当作字节数。不估计SQLite varint、索引或页成本，namespace公共键沿现有预算约定，数据目录/卷空间继续独立门禁。

计量和写入共用编码辅助对象；观察与gap恰有一个，完整观察沿Core原validate/encode和1024字节上限。基础节点成本加旁表成本checked，预算正好相等可准入，差1字节拒绝。覆盖完整观察、每类gap与互斥错误；不放宽节点、fence、取消、时限或容量。

本变更修复预算语义，不证明扫描性能回退根因或生产验收完成。

## 当前验证

- 旧计费回归：2失败/1通过，失败值包含gap实际28字节被计为1024字节。
- 修复后macOS计费及边界回归：4通过；实际Unix旁表发布、末检拒绝和事务回滚：3通过。
- Store/Engine本机all-targets Clippy通过。Linux Engine编译检查通过；Linux计费回归4通过、实际旁表回归3通过。
- 新生产调用位于Linux条件分支，本机测试不能替代Linux运行验收；未声明整体生产就绪。

- Linux真实worker运行：gap额外计费拒绝发布、raw locator预算拒绝发布、大文件容量不消耗元数据预算，三个Engine回归各1通过。没有新增完整扫描精确预算边界验收的声明。
