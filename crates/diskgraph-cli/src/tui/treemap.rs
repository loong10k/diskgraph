//! 大小映射和有预算的递归地图绘制。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::palette::{color_from, fit, mix_toward};
use super::{Browser, Entry, PAGE_SIZE};
use crate::tui_frame_reader::TuiFrameReader;
use diskgraph_core::treemap::{self, Rect as MapRect, Weighted};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// 按子节点大小绘制地图，使用与其他 CLI 展示面相同的色板。
/// 参数：frame/browser/reads 为同一画布、状态和账本，area 指定地图区域。
/// 返回：无，必要子层读取仍继承整帧授权和原期限。
pub(super) fn draw_map(
    frame: &mut ratatui::Frame<'_>,
    browser: &Browser,
    reads: &mut TuiFrameReader<'_>,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" disk usage · {}", browser.revision));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let entries = browser.ordered();
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(if browser.hidden() > 0 {
                format!(
                    "{} children are all under the threshold (press m)",
                    browser.hidden()
                )
            } else {
                "nothing indexed here".to_owned()
            })
            .style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }
    draw_children(frame, browser, reads, &entries, inner, 0);
}

/// 将当前目录子节点绘入区域；可辨识的较大目录在共同预算内递归展开。
fn draw_children(
    frame: &mut ratatui::Frame<'_>,
    browser: &Browser,
    reads: &mut TuiFrameReader<'_>,
    entries: &[&Entry],
    area: Rect,
    depth: usize,
) {
    let width = area.width.saturating_sub(1) as f64;
    let height = area.height.saturating_sub(1) as f64;
    if entries.is_empty() || width < 4.0 || height < 1.0 {
        return;
    }
    let items: Vec<Weighted> = entries
        .iter()
        .map(|entry| Weighted {
            id: entry.id,
            weight: entry.size_bytes as f64,
        })
        .collect();
    let placed = treemap::squarify(&items, MapRect::new(1.0, 0.0, width, height));
    let largest = entries
        .iter()
        .map(|entry| entry.size_bytes)
        .max()
        .unwrap_or(1)
        .max(1);
    let by_id: std::collections::HashMap<u64, &Entry> =
        entries.iter().map(|entry| (entry.id, *entry)).collect();

    for item in placed {
        let Some(entry) = by_id.get(&item.id).copied() else {
            continue;
        };
        let x = item.rect.x as u16;
        let y = item.rect.y as u16;
        if x >= area.width || y >= area.height {
            continue;
        }
        let cell = Rect {
            x: area.x + x,
            y: area.y + y,
            width: (item.rect.width as u16)
                .max(1)
                .min(area.width.saturating_sub(x)),
            height: (item.rect.height as u16)
                .max(1)
                .min(area.height.saturating_sub(y)),
        };
        // 亮度体现相对大小；较大节点先呈现饱和颜色，便于先看地图再读标签。
        let share = entry.size_bytes as f64 / largest as f64;
        let base = color_from(entry.category.as_deref());
        let color = mix_toward(base, Color::Black, (1.0 - share) * 0.45);
        // 过窄块保持面积和颜色，避免无法辨识的名称占用画面。
        if cell.width >= 8 {
            let label = fit(
                &format!("{} {}", entry.name, treemap::human_bytes(entry.size_bytes)),
                cell.width,
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    label,
                    Style::default().fg(Color::White).bg(color),
                ))),
                cell,
            );
        } else {
            frame.render_widget(Block::default().style(Style::default().bg(color)), cell);
        }
        if browser.selected_id == Some(entry.id) {
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White)),
                cell,
            );
        }
        if depth < MAX_PAINT_DEPTH && entry.has_children && cell.width > 12 && cell.height > 4 {
            // 整帧共享工作预算；预算耗尽后仍保留可进入的父块。
            if let Some(mut layer) = reads.load_layer(entry.id, PAGE_SIZE) {
                if let Some(pseudonyms) = browser.pseudonyms.as_ref() {
                    for child in &mut layer.children {
                        child.name = pseudonyms.label_for(child.id);
                    }
                }
                // 单子节点的同色块不增加信息，保留父块并由 Enter 展示内部。
                if layer.children.len() > 1 {
                    let deeper: Vec<&Entry> = layer.children.iter().collect();
                    draw_children(frame, browser, reads, &deeper, cell, depth + 1);
                }
            }
        }
    }
}

/// 自动绘制的最大嵌套深度；更深目录由 Enter 导航打开。
const MAX_PAINT_DEPTH: usize = 2;
