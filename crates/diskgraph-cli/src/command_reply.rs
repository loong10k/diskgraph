//! 单次命令的原公开响应交付；不承担恢复责任。
use diskgraph_engine::EngineError;
use std::io::Write;

/// 参数：原业务结果、JSON行及模式、原输出端、错误尝试标记；返回：同一业务结果。
/// 原响应须在执行栈结束前交付，后续恢复只持有资源责任。
pub(crate) fn deliver<T>(
    outcome: Result<T, EngineError>,
    lines: Vec<String>,
    json: bool,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    error_attempted: &mut bool,
) -> Result<T, EngineError> {
    let response = (|| -> std::io::Result<()> {
        if json {
            for line in lines {
                writeln!(stdout, "{line}")?;
            }
        }
        stdout.flush()
    })();
    match outcome {
        Err(error) => {
            // 原业务失败优先；即使公开端已断开，也禁止外层在恢复结束后重发正文。
            *error_attempted = true;
            if response.is_ok() {
                if json {
                    let _ = writeln!(stdout, "{}", crate::error_reply::line(&error))
                        .and_then(|()| stdout.flush());
                } else {
                    let _ = writeln!(stderr, "{error}").and_then(|()| stderr.flush());
                }
            }
            Err(error)
        }
        Ok(value) => match response {
            Ok(()) => Ok(value),
            Err(error) => {
                // 成功业务的公开交付失败不能变成成功，也不能在公共EOF之后再次尝试输出。
                *error_attempted = true;
                Err(error.into())
            }
        },
    }
}

#[cfg(test)]
#[path = "command_reply_tests.rs"]
mod tests;
