//! 与其他展示面共享的颜色映射、亮度和标签裁剪。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use crate::html::{PALETTE, color_for};
use ratatui::style::Color;

/// 按相对大小将颜色混向另一颜色，保留既有亮度斜坡。
/// 参数：from/to 为颜色，amount 为混合比例。返回：原 RGB 混合或非 RGB 主题颜色。
pub(super) fn mix_toward(from: Color, to: Color, amount: f64) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| -> u8 {
        (f64::from(a) * (1.0 - amount) + f64::from(b) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let (r2, g2, b2) = match to {
        Color::Rgb(r, g, b) => (r, g, b),
        // Any other colour is treated as the dark end of the ramp, which is
        // what the share-to-brightness scale actually wants.
        _ => (0, 0, 0),
    };
    match from {
        Color::Rgb(r1, g1, b1) => Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2)),
        // Only RGB can be blended; a themed colour stays as it is.
        single => single,
    }
}

/// 将标签裁剪到单元宽度，避免窄块中的名称换行。
/// 参数：label 为原标签，width 为字符宽度。返回：原文字或含省略号的裁剪标签。
pub(super) fn fit(label: &str, width: u16) -> String {
    let width = width as usize;
    if label.chars().count() <= width {
        return label.to_owned();
    }
    if width <= 1 {
        return "…".to_owned();
    }
    let mut out: String = label.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// 将已有分类色板转换为终端 RGB 颜色。
/// 参数：category 为可选分类。返回：既有分类色或默认色。
pub(super) fn color_from(category: Option<&str>) -> Color {
    let hex = color_for(category);
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x4a5462);
    Color::Rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

/// 获取公共色板的非空分类图例。
/// 参数：无。返回：与其他展示面一致的分类和十六进制颜色列表。
#[allow(dead_code, reason = "the legend is shared with documentation tools")]
pub fn legend() -> Vec<(&'static str, &'static str)> {
    PALETTE
        .iter()
        .filter(|(name, _)| !name.is_empty())
        .copied()
        .collect()
}
