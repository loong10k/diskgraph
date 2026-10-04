//! 单帧缓冲、有限授权和最终提交，保留既有执行顺序。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::Layer;
use crate::tui_frame_reader::TuiFrameReader;
use crate::tui_request::{TUI_DISPLAY_BYTES, TuiRequest};
use diskgraph_engine::{EngineError, RevisionDisplayCompletion};
use ratatui::Terminal;

/// 先绘入终端的内存缓冲，整帧最终授权通过后才提交到后端。
/// 参数：terminal 为现有终端，request 为真实身份，cached_layer 为既有页面，paint 绘制本帧。
/// 返回：提交成功或原错误；真实撤权、到期和存储失败不提交缓冲。
pub(super) fn draw_authorized_frame<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    request: &TuiRequest<'_>,
    cached_layer: &Layer,
    paint: impl FnOnce(&mut ratatui::Frame<'_>, &mut TuiFrameReader<'_>),
) -> Result<(), EngineError>
where
    B::Error: std::fmt::Display,
{
    terminal
        .autoresize()
        .map_err(|error| EngineError::Io(std::io::Error::other(error.to_string())))?;
    request.engine.with_authorized_revision_display_reader(
        request.revision,
        request.principal,
        request.authorizer,
        50,
        |reader, snapshot, deadline| {
            let bytes = TuiFrameReader::display_bytes(cached_layer);
            if bytes > TUI_DISPLAY_BYTES {
                return Err(EngineError::Business(
                    diskgraph_core::BusinessError::BudgetExceeded,
                ));
            }
            let mut reads = TuiFrameReader::new(
                reader,
                snapshot,
                deadline,
                cached_layer.children.len() + 1,
                bytes,
            );
            let mut frame = terminal.get_frame();
            paint(&mut frame, &mut reads);
            if let Some(error) = reads.take_error() {
                return Err(error);
            }
            // 只接受 paint 中已经显示的截断状态；此处不临时制造未显示的 late partial。
            Ok(if reads.truncation_reason.is_some() {
                RevisionDisplayCompletion::Truncated
            } else {
                RevisionDisplayCompletion::Complete
            })
        },
    )?;
    terminal
        .apply_buffer()
        .map_err(|error| EngineError::Io(std::io::Error::other(error.to_string())))?;
    Ok(())
}
