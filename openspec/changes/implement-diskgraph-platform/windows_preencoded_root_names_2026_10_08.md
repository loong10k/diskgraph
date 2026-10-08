# Windows根链名称预编码候选

沿用PF-06及D31。原400003次根检查对每个固定组件重复`encode_wide`和分配临时Vec；预先编码根租约的固定原生名称，复用不可变UTF-16单组件输入，保留每次NtCreateFile及全部FileState捕获、任务check、名称/身份/重解析/同卷检查。不会缓存句柄查询结果或授权。

验收要求：原OsStr路径与UTF-16路径返回同一完整128位身份；非UTF-8（未配对surrogate）原生名称无损；空名称、点、父路径、分隔符、冒号、NUL及超长输入仍在系统调用前拒绝。保留根/祖先替换、目录时间变化、取消、撤权及原生扫描所有回归。原生函数输入缓冲在同步调用期间借用，NtCreateFile仅使用输入ObjectAttributes，不得写入借用名称。

本机无Windows编译目标，Windows行为红/绿灯与性能测量暂不可执行，不将macOS源码检查当作原生通过。候选必须进入下一同SHA Windows CI；没有测得时间/RSS收益前，不关闭300秒或根校验成本问题。参考NtCreateFile官方输入参数契约：https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-zwcreatefile 。

候选已实现：根租约保存预编码Vec<Vec<u16>>；原OsStr入口仍执行同一bounded编码并路由到同一原生函数。预编码入口仍验证单组件边界，不保存或复用任何查询结果。新增Windows实际文件回归，比较原入口/新入口完整WindowsFileState，涵盖正常目录、中文名称、未配对surrogate以及八种非法输入；该测试已接入现有全workspace门禁，尚待Windows宿主执行。本机全平台源码AST门禁6/6、宿主all-targets Clippy拒绝警告、fmt与diff检查通过；证据见`docs/benchmarks/windows_preencoded_root_names_2026_10_08/receipt.json`。
