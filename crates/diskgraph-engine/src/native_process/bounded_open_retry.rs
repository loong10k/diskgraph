//! 保持原检查与参数的有限原生打开重试；来源：Linux openat2(2)，无 Java 对等对象。
use diskgraph_core::ProcessEvidenceFailureCode as Failure;

/// 参数：原检查、同一打开操作及唯一允许重试的 errno；返回：句柄或现场错误。
pub(super) fn bounded_open_retry<T>(
    check: &dyn Fn() -> Result<(), Failure>,
    open: &mut dyn FnMut() -> std::io::Result<T>,
    retry_errno: i32,
) -> Result<std::io::Result<T>, Failure> {
    // 上限同时约束调用次数；不靠新的墙钟期限延长原请求预算。
    for attempt in 0..8 {
        check()?;
        let result = open();
        check()?;
        if !matches!(&result, Err(error) if error.raw_os_error() == Some(retry_errno))
            || attempt == 7
        {
            return Ok(result);
        }
        std::thread::yield_now();
    }
    unreachable!("the last attempt always returns")
}
