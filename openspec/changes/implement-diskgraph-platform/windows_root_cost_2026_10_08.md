# Windows 根链成本定位与原CI终态

整体未生产就绪，不关闭产品监督、真实FileProvider、Windows300秒或最终平台父项。

## 仅诊断的实现

保留原Windows根链校验全部逐组件capture/名称重新打开/静态身份比较/check顺序。原validate_held_root函数体移入inner，逐字节摘要相同；显式诊断开关在根租约创建时取样一次。默认直接调用原inner；开启时用单调时钟包裹原调用，并以两个AtomicU64饱和累计已返回调用数及纳秒。原错误保持Result，panic原payload自然传播，不计为已完成调用。模块保持Send/Sync，没有锁、路径缓存或允许授权缓存。

staging阶段结束仅输出固定纯数值：windows_root_validation时间和root calls/chain_handles。累计含租约首次校验至staging结束，不含后续发布终检，不是纯内核调用或CPU成本；它与observations_and_encoding重叠，不能相加。取消/失败在此前退出可能没有此分项，禁止补造。负载转存保留原128KiB后缀和有限记录，新增标签/字段严格数字全匹配，未知/路径/额外后缀仍拒绝。没有减少200k文件、6项检查、正式300秒期限或原清理。

本机ARM macOS独立编译纯std计数器3/3；parser新增目标先行为失败，修正后18/18；脚本全量298项，297通过、1项Windows原生测试跳过；Engine all-targets Clippy、fmt、diff、OpenSpec strict通过。本机仅aarch64-apple-darwin目标，不宣称Windows cfg分支已编译或运行；原生新源码分项数据仍待CI。

## 原616958e终态

CI37759437051全部23任务终态：19成功、4失败。Windows stable与MSRV的完整job均成功，Engine670通过/4忽略、Store354通过/5忽略；包含原生步骤、workspace和后续migration检查。三个macOS Rust job失败均是旧sleep竞争目标，Windows包失败为正式200k/300秒。其他平台包/FFI宿主等各自成功不覆盖这些失败，不重标为本次新源码证据。已保存原API及两份Windows完整日志，随后成批推送本地已验证改动。

FILE_ID_INFO参考：https://learn.microsoft.com/en-us/windows/desktop/api/winbase/ns-winbase-file_id_info 。这里只参考卷与128位ID的句柄身份语义，没有据此删减任何根链校验。

证据见docs/benchmarks/windows_root_cost_2026_10_08/summary.json；gzip解压摘要逐项记录。
