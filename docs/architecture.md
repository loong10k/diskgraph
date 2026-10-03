# DiskGraph 架构文档 / Architecture

本文已迁移为完整的中英文独立文档；保留此入口兼容历史链接。

This document has moved to complete, independent language editions. This entry remains for existing links.

- [English](DiskGraph-Architecture.md)
- [简体中文](DiskGraph-Architecture.zh_CN.md)

正式需求与验收仍以 [OpenSpec](../openspec/changes/implement-diskgraph-platform/proposal.md) 为唯一事实源。

The [OpenSpec change](../openspec/changes/implement-diskgraph-platform/proposal.md) remains the sole formal requirements and acceptance source.

Updated / 更新：2026-09-28。

全平台实施与运行证据 / Full-platform implementation and runtime evidence: [English](production-readiness-full-platform-2026-10-02.md) · [简体中文](production-readiness-full-platform-2026-10-02.zh-CN.md)。

FFI 源码边界 / FFI source boundaries：API、realm、扫描协调、JobHandle 与共享 JobState 分文件；标准固定 include 保留旧 UniFFI 词法路径和校验值，不新增状态 owner。Windows 补充观测从保留父句柄验证当前名称及完整身份；有限复核不保证原子命名空间，pinned 树遍历仍按路径。D31 原生根替换回归失败、修复待新的原生 CI，见全平台实施记录。
