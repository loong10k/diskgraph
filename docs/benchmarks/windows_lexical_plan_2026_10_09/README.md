# Windows 原生路径规划回归

实际执行环境为用户台式机 E:\workspaces\workspace-loong10k\diskgraph，Windows x86_64 MSVC，Rust 1.99.0。

红灯使用当前生产实现与新增测试：20 次文件观测产生 40 次词法解析，断言失败。修复只保留注册根的不可变词法计划，每个请求仍解析原生请求路径，仍执行根链名称、卷及完整身份检查和原请求取消、授权、fencing 检查。

原生扫描回归 14 passed / 0 failed / 0 ignored；严格 Engine all-targets Clippy 退出 0。green_source.json 的三份源码 SHA-256 已与本地提交源码逐一核对。

该证据仅证明词法重复工作减少及所列回归。不能证明 200k 性能门禁、完整 workspace、CLI/MCP、受保护安装、前端恢复监督链或全平台生产验收已经完成。完整 workspace 在台式机另以 f07f463b 运行，结果应独立记录。
