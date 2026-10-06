//! 成功或断言panic后都等待原runner，源目录不能先于扫描任务销毁。
use diskgraph_engine::JobRunner;

/// 保活扫描源并实际等待后台任务调度线程结束的测试owner。
/// 来源：原生 Rust PF-06 MCP回归；无 Java 对等对象。
pub(crate) struct McpTestRunner {
    runner: Option<JobRunner>,
    _source: tempfile::TempDir,
}

impl McpTestRunner {
    /// 参数：runner 为原服务启动的句柄，source 为原扫描源目录。
    /// 返回：唯一保活责任；析构不会把设置停止标志当作线程已结束。
    pub(crate) fn new(runner: JobRunner, source: tempfile::TempDir) -> Self {
        Self {
            runner: Some(runner),
            _source: source,
        }
    }
}

impl Drop for McpTestRunner {
    fn drop(&mut self) {
        if let Some(runner) = self.runner.take()
            && let Err(payload) = runner.stop_and_join()
        {
            if std::thread::panicking() {
                // 原前台断言已经失败，保留它的unwind，后台失败仍写入原日志。
                eprintln!("MCP test runner panicked during foreground failure recovery");
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

#[cfg(test)]
mod tests;
