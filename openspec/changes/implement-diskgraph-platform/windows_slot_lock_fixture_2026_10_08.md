# Windows 容量槽测试强制锁语义修正

事实源：implement-diskgraph-platform；本次只修正测试夹具，不改变生产容量、身份或回收契约。

当前原生 MSRV CI 37664417193/job112939850186 的 Engine 库 625通过、1失败、4忽略。失败为新 RESERVED owner 仍持独占字节锁时，测试从第二句柄读取记录返回 OS33。原始日志保留于 docs/benchmarks/windows_slot_lock_fixture_2026_10_08。

验收保持：旧 owner 重复退休不得改变新槽；新 owner activate 从原锁精确检查 RESERVED，verify_active 精确检查 ACTIVE。Unix 保留第二句柄记录比较；Windows 精确要求 ERROR_LOCK_VIOLATION 33，双方额外要求竞争认领 Busy。未放宽生产错误或取消锁。

本机 macOS 相关9项通过、Rust2024格式检查通过、两路审查APPROVE/CLEAR。修复后 Windows 实际 CI 尚待完成，不关闭平台父项，不声明生产就绪。
