# 有限 FUSE 修复审查

代码审查 APPROVE，架构审查 CLEAR；仅针对已复现 FUSE 路径。原 O_PATH fd 上 fstatfs 拒绝、逐组件 no-follow、原 fd 存活及 dev/ino 复验支持对象绑定，最终数据打开仍执行 OS 权限检查。

兼容性：FUSE 已驻留文件也拒绝；缺失/受限 procfs 保留 I/O 错误，无普通路径回退。此修复不证明 metadata/lookup 不召回，也不证明非 FUSE、网络文件系统或 overlay 下层 provider 可安全物化。通用 Linux 能力与 CT-02 全平台验收仍未完成。
