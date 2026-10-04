//! 状态栏、地图和选择详情的整帧布局。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::treemap::draw_map;
use super::{Browser, PAGE_SIZE};
use crate::tui_frame_reader::TuiFrameReader;
use diskgraph_core::treemap;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

/// 组合一帧状态栏、地图和选择详情，在真实绘制提示前标记耗尽状态。
/// 参数：frame/browser/reads 为同一现有帧、状态和读取账本。返回：无，仅写内存画布。
pub(super) fn draw(
    frame: &mut ratatui::Frame<'_>,
    browser: &Browser,
    reads: &mut TuiFrameReader<'_>,
) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(rows[1]);

    draw_map(frame, browser, reads, columns[0]);
    draw_details(frame, browser, columns[1]);
    // 最后一个嵌套读取之后的绘图也可能耗尽读期；先标记，再真实绘制提示。
    reads.can_continue();
    frame.render_widget(status_line(browser, reads), rows[0]);
}

fn status_line<'a>(browser: &'a Browser, reads: &TuiFrameReader<'_>) -> Paragraph<'a> {
    let layer = browser.current();
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", layer.name),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            reads
                .truncation_reason
                .map(|reason| format!("  nested view truncated: {reason} · Enter to inspect "))
                .unwrap_or_default(),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "read {} B · view {} B  ",
                reads.preparation_bytes(),
                reads.retained_display_bytes()
            ),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            treemap::human_bytes(layer.total_bytes),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("  {} files", layer.total_files),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!(
                "  page {}{}{}",
                layer.offset / PAGE_SIZE as u64 + 1,
                if layer.offset > 0 {
                    " · p previous"
                } else {
                    ""
                },
                if layer.has_more { " · n next" } else { "" },
            ),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            format!(
                "   {}{}",
                browser.status,
                match (browser.hidden(), browser.min_share) {
                    (0, _) => String::new(),
                    (hidden, share) if share > 0.0 => {
                        format!("  · {hidden} smaller than {:.1}% hidden", share * 100.0)
                    }
                    (hidden, _) => format!("  · {hidden} empty"),
                }
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

fn draw_details(frame: &mut ratatui::Frame<'_>, browser: &Browser, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" selection ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let entries = browser.ordered();
    let Some(entry) = entries
        .iter()
        .find(|entry| Some(entry.id) == browser.selected_id)
        .or_else(|| entries.first())
    else {
        frame.render_widget(
            Paragraph::new("nothing selected").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    };
    let mut lines = vec![
        Line::from(Span::styled(
            entry.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!(
            "size       {}",
            treemap::human_bytes(entry.size_bytes)
        )),
        Line::from(format!("files      {}", entry.files)),
        Line::from(format!("kind       {}", entry.kind)),
        Line::from(format!(
            "category   {}",
            entry.category.clone().unwrap_or_else(|| "-".into())
        )),
        Line::from(format!("children   {}", entry.has_children)),
    ];
    if entry.read_error {
        lines.push(Line::from(Span::styled(
            "read error - the size below this point is unknown",
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        browser.status.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}
