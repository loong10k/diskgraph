# 冻结 Linux 基线契约夹具

两个 gzip 文件解压后与提交 `2a2f8281f9f211b6632bdb26afdcd4a4fb21a44d` 的 `crates/diskgraph-engine/src/native_process/` 下同名源码逐字节一致，gzip 时间戳固定为零。只供 Python 诊断适配器测试，避免浅克隆缺少历史对象导致环境失败；不会编入产品或替代原生负载验收。

- linux_scan_namespace.rs：SHA256 `4787a0d69af24d0914e8a394716a4ea2dac3ba4b9b029fac3864e60701fbf9bc`
- linux_open.rs：SHA256 `b569eda8885214eb66f9fc026daa9751e504f88ac6cb7f347a5f2e0441d635d8`
