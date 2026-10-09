# Linux GNU包ABI门禁落地

继实际glibc2.36加载失败后，固定2.17门禁进入真实包入口、私有Linux脚本及原生CI。检查三份源文件、staging和解包字节；源文件不合格时不能覆盖原归档。目标架构及同一打开句柄的有限读取、身份复验和摘要绑定。静态JSON明确runtime_qualification=false。

原HEAD包入口行为红灯：一个测试的CLI/MCP/worker三个超标子例均未拒绝；最初Mock返回值不可JSON序列化等测试配置错误独立存档，不冒充行为红灯。修复后ABI13、包组成11、文件准入5、部署绑定7、Rust部署契约6通过。独立运行9项原检查也通过；workflow语法（ShellCheck关闭）、覆盖门禁与OpenSpec严格校验通过。

真实98455bcf ARM64包摘要与版本需求见real_images.json：CLI/MCP2.39、worker2.34。新实际包入口exit1，尚未创建output目录、尚未执行镜像即拒绝2.17声明。此为真实制品静态拒绝证据，不是ARM业务运行通过。

旧基线SDK/容器安装授权仍待回复；没有下载或安装。当前源码在旧glibc环境的重建、启动、隔离业务、内核安全能力、全平台与性能门禁仍未完成。本轮新门禁尚未执行原生CI。
