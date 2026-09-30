//! Treemap layout and text rendering, shared by the three visible surfaces
//! (terminal TUI, single-file HTML, and the agent-facing text format).
//!
//! The layout is the classic squarified treemap (Bruls, Huizing & van
//! Wijk): children are laid out in descending size order into rows whose
//! aspect ratios stay near one, so a directory's children read as areas
//! proportional to their bytes instead of a bar chart pretending to be a
//! map. Everything here is a pure function over plain numbers - the same
//! layout then drives a character grid, a terminal canvas, and a browser
//! canvas without any of them re-deciding what belongs where.

/// An axis-aligned rectangle in abstract layout units (cells for the text
/// renderer, pixels for the browser, terminal cells for the TUI).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn area(&self) -> f64 {
        (self.width * self.height).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

/// One entry to place: a stable identity plus the weight that decides its
/// area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weighted {
    /// Caller-assigned identity, returned unchanged so a click or a key
    /// press can be mapped back to a node.
    pub id: u64,
    pub weight: f64,
}

/// A weighted entry with the rectangle it occupies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub id: u64,
    pub rect: Rect,
}

/// Lays out `items` inside `area` so that each rectangle's area is
/// proportional to its weight and no two rectangles overlap.
///
/// Items are placed in descending weight order, which makes the result
/// deterministic: the same children always produce the same map, and the
/// biggest child is always the first thing a reader's eye lands on.
pub fn squarify(items: &[Weighted], area: Rect) -> Vec<Placed> {
    let mut ordered: Vec<Weighted> = items
        .iter()
        .copied()
        .filter(|item| item.weight > 0.0 && item.id != 0 || item.weight > 0.0)
        .collect();
    ordered.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let total: f64 = ordered.iter().map(|item| item.weight).sum();
    if total <= 0.0 || area.is_empty() {
        return Vec::new();
    }
    let scale = area.area() / total;
    let mut remaining = ordered;
    let mut placed = Vec::with_capacity(remaining.len());
    let mut free = area;

    while !remaining.is_empty() {
        let side = free.width.min(free.height);
        if side <= 0.0 {
            break;
        }
        // Fill one row: keep taking items while doing so keeps the row's
        // aspect ratio at or above the worst item's.
        let mut row: Vec<Weighted> = Vec::new();
        let mut row_weight = 0.0;
        let mut best_ratio = f64::INFINITY;
        while let Some(next) = remaining.first().copied() {
            let candidate_weight = row_weight + next.weight;
            let ratio = worst_ratio(&row, next, candidate_weight, side, scale);
            if row.is_empty() || ratio <= best_ratio {
                best_ratio = ratio;
                row.push(next);
                row_weight = candidate_weight;
                remaining.remove(0);
            } else {
                break;
            }
        }
        if row.is_empty() {
            break;
        }
        let row_area = row_weight * scale;
        // Lay the strip along the free area's SHORT side: on a wide area the
        // row runs left-to-right and its items are vertical bars whose width
        // is proportional to their weight.
        let horizontal = free.width >= free.height;
        let short_side = if horizontal { free.height } else { free.width };
        let row_thickness = if short_side > 0.0 {
            (row_area / short_side).clamp(0.0, short_side)
        } else {
            0.0
        };
        let strip_length = if horizontal { free.width } else { free.height };
        let row_start = if horizontal { free.x } else { free.y };
        let row_len = row.len();
        let mut offset = row_start;
        for (index, item) in row.into_iter().enumerate() {
            // The last item absorbs the accumulated rounding slack so the row
            // reaches the end of the strip exactly.
            let extent = if index + 1 == row_len {
                (strip_length - (offset - row_start)).max(0.0)
            } else {
                (item.weight * scale / row_thickness).clamp(0.0, strip_length)
            };
            let rect = if horizontal {
                Rect::new(offset, free.y, extent, row_thickness)
            } else {
                Rect::new(free.x, offset, row_thickness, extent)
            };
            placed.push(Placed { id: item.id, rect });
            offset += extent;
        }
        free = if horizontal {
            Rect::new(
                free.x + row_thickness,
                free.y,
                (free.width - row_thickness).max(0.0),
                free.height,
            )
        } else {
            Rect::new(
                free.x,
                free.y + row_thickness,
                free.width,
                (free.height - row_thickness).max(0.0),
            )
        };
    }
    placed
}

/// The worst (largest) aspect ratio the row would have with `next` added.
fn worst_ratio(row: &[Weighted], next: Weighted, row_weight: f64, side: f64, scale: f64) -> f64 {
    let area = row_weight * scale;
    if area <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let thickness = (area / side).min(side);
    if thickness <= 0.0 {
        return f64::INFINITY;
    }
    let mut worst = 0.0_f64;
    for item in row {
        worst = worst.max(
            (item.weight * scale / (thickness * thickness))
                .abs()
                .max(0.0)
                .powf(0.5),
        );
    }
    let next_extent = next.weight * scale / (thickness * thickness);
    worst = worst.max(next_extent.sqrt());
    worst
}

/// One row of the text treemap: an identity, a label, and a weight.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRow {
    pub id: u64,
    pub name: String,
    pub size_bytes: u64,
    pub files: u64,
    /// Optional right-hand annotation, e.g. the full path or a category.
    pub note: Option<String>,
    /// Rendered dimmer: still visible, but not a candidate for attention.
    pub muted: bool,
}

/// Fractional block glyphs, darkest first. A cell's density encodes the
/// share of the row it stands for, so the map still reads as an area
/// rather than a list.
const BLOCKS: [char; 8] = ['█', '▉', '▊', '▋', '▌', '▍', '▎', '▏'];

/// Renders rows as a text treemap `width` columns wide.
///
/// The caller supplies rows already sorted largest first; each row's bar is
/// drawn to scale against the largest row, so the relative sizes are
/// readable in a terminal, a code review, or an agent transcript where a
/// canvas cannot be shown. The summary line states the denominator, so a
/// truncated view never reads as a complete one.
pub fn render_text(rows: &[TextRow], width: usize) -> String {
    let width = width.max(24);
    let mut out = String::new();
    let largest = rows
        .iter()
        .map(|row| row.size_bytes)
        .max()
        .unwrap_or(0)
        .max(1);
    let total: u64 = rows.iter().map(|row| row.size_bytes).sum();
    let total_files: u64 = rows.iter().map(|row| row.files).sum();

    // Reserve room for the label so a bar never pushes text off the edge.
    let label_room = rows
        .iter()
        .map(|row| row.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(28);
    // Reserve the numeric columns up front so a narrow terminal still fits:
    // label + bar + 9 + 7 + separators.
    let label_room = label_room.min(width.saturating_sub(24).max(8));
    let bar_room = width.saturating_sub(label_room + 24).max(4);

    out.push_str(&format!(
        "{:>label$}  {:>9}  {:>7}\n",
        "SIZE",
        "TOTAL",
        "FILES",
        label = label_room.max(4)
    ));
    out.push_str(&("─".repeat(width.min(100)) + "\n"));

    for row in rows {
        let share = row.size_bytes as f64 / largest as f64;
        let full = (share * bar_room as f64).round() as usize;
        let fraction = (share * bar_room as f64) - full as f64;
        let mut bar = String::new();
        for _ in 0..full.min(bar_room) {
            bar.push(BLOCKS[0]);
        }
        if full < bar_room && fraction > 0.0 {
            let index = ((1.0 - fraction) * (BLOCKS.len() - 1) as f64).round() as usize;
            bar.push(BLOCKS[index.min(BLOCKS.len() - 1)]);
        }
        let name = truncate_label(&row.name, label_room);
        let note = row.note.clone().unwrap_or_default();
        let marker = if row.muted { " " } else { "▎" };
        out.push_str(&format!(
            "{marker}{name:<label_width$} {blocks:<block_width$} {size:>9} {files:>7}  {note}\n",
            size = human_bytes(row.size_bytes),
            files = row.files,
            name = name,
            blocks = bar,
            label_width = label_room,
            block_width = bar_room,
            marker = marker,
            note = note,
        ));
    }
    out.push_str(&format!(
        "\n{} entries · {} in {} files\n",
        rows.len(),
        human_bytes(total),
        total_files
    ));
    out
}

fn truncate_label(name: &str, room: usize) -> String {
    let count = name.chars().count();
    if count <= room {
        return name.to_owned();
    }
    let keep = room.saturating_sub(1);
    let mut out: String = name.chars().take(keep).collect();
    out.push('…');
    out
}

/// Human-readable byte size with a unit, e.g. `1.25 GiB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(placed: &[Placed]) -> Vec<u64> {
        let mut out: Vec<u64> = placed.iter().map(|item| item.id).collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn every_weighted_child_gets_a_rectangle() {
        let items = vec![
            Weighted {
                id: 1,
                weight: 50.0,
            },
            Weighted {
                id: 2,
                weight: 30.0,
            },
            Weighted {
                id: 3,
                weight: 20.0,
            },
        ];
        let placed = squarify(&items, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(ids(&placed), vec![1, 2, 3]);
    }

    #[test]
    fn areas_track_the_weights() {
        let items = vec![
            Weighted {
                id: 1,
                weight: 60.0,
            },
            Weighted {
                id: 2,
                weight: 25.0,
            },
            Weighted {
                id: 3,
                weight: 15.0,
            },
        ];
        let area = Rect::new(0.0, 0.0, 200.0, 100.0);
        let placed = squarify(&items, area);
        let used: f64 = placed.iter().map(|item| item.rect.area()).sum();
        // Children fill the parent area up to floating-point slack.
        assert!(
            (used - area.area()).abs() < 1.0,
            "used {used} of {}",
            area.area()
        );
        for item in &placed {
            let source = items
                .iter()
                .find(|candidate| candidate.id == item.id)
                .unwrap();
            let expected = source.weight / 100.0 * area.area();
            assert!(
                (item.rect.area() - expected).abs() < 1.0,
                "child {} area {} vs expected {expected}",
                item.id,
                item.rect.area()
            );
        }
    }

    #[test]
    fn rectangles_never_overlap() {
        let items: Vec<Weighted> = (1..=12)
            .map(|id| Weighted {
                id,
                weight: (13 - id) as f64,
            })
            .collect();
        let placed = squarify(&items, Rect::new(0.0, 0.0, 80.0, 24.0));
        for (index, first) in placed.iter().enumerate() {
            for second in placed.iter().skip(index + 1) {
                let overlap_x = (first.rect.x + first.rect.width)
                    .min(second.rect.x + second.rect.width)
                    - first.rect.x.max(second.rect.x);
                let overlap_y = (first.rect.y + first.rect.height)
                    .min(second.rect.y + second.rect.height)
                    - first.rect.y.max(second.rect.y);
                assert!(
                    overlap_x <= 0.01 || overlap_y <= 0.01,
                    "{} overlaps {}",
                    first.id,
                    second.id
                );
            }
        }
    }

    #[test]
    fn rectangles_stay_inside_their_parent() {
        let items: Vec<Weighted> = (1..=8)
            .map(|id| Weighted {
                id,
                weight: id as f64,
            })
            .collect();
        let area = Rect::new(3.0, 5.0, 40.0, 20.0);
        for placed in squarify(&items, area) {
            assert!(placed.rect.x >= area.x - 0.01);
            assert!(placed.rect.y >= area.y - 0.01);
            assert!(placed.rect.x + placed.rect.width <= area.x + area.width + 0.01);
            assert!(placed.rect.y + placed.rect.height <= area.y + area.height + 0.01);
        }
    }

    #[test]
    fn a_lone_child_takes_the_whole_area() {
        let placed = squarify(
            &[Weighted {
                id: 7,
                weight: 10.0,
            }],
            Rect::new(0.0, 0.0, 30.0, 10.0),
        );
        assert_eq!(placed.len(), 1);
        assert!((placed[0].rect.area() - 300.0).abs() < 0.01);
    }

    #[test]
    fn zero_weights_and_empty_areas_place_nothing() {
        assert!(squarify(&[], Rect::new(0.0, 0.0, 10.0, 10.0)).is_empty());
        assert!(
            squarify(
                &[Weighted { id: 1, weight: 0.0 }],
                Rect::new(0.0, 0.0, 10.0, 10.0)
            )
            .is_empty()
        );
        assert!(
            squarify(
                &[Weighted { id: 1, weight: 5.0 }],
                Rect::new(0.0, 0.0, 0.0, 10.0)
            )
            .is_empty()
        );
    }

    #[test]
    fn the_same_children_always_lay_out_the_same_way() {
        let items = vec![
            Weighted {
                id: 4,
                weight: 10.0,
            },
            Weighted {
                id: 3,
                weight: 30.0,
            },
            Weighted {
                id: 2,
                weight: 30.0,
            },
            Weighted { id: 1, weight: 5.0 },
        ];
        let area = Rect::new(0.0, 0.0, 64.0, 20.0);
        let first = squarify(&items, area);
        let mut shuffled = items.clone();
        shuffled.reverse();
        let second = squarify(&shuffled, area);
        assert_eq!(first, second, "layout must not depend on input order");
    }

    #[test]
    fn the_text_map_marks_the_largest_row_with_a_full_block() {
        let rows = vec![
            TextRow {
                id: 1,
                name: "workspaces".into(),
                size_bytes: 100 * 1024 * 1024 * 1024,
                files: 2_400_000,
                note: Some("code".into()),
                muted: false,
            },
            TextRow {
                id: 2,
                name: "cache".into(),
                size_bytes: 1024 * 1024 * 1024,
                files: 12_000,
                note: None,
                muted: false,
            },
        ];
        let text = render_text(&rows, 80);
        assert!(text.contains("workspaces"));
        assert!(text.contains("100 GiB"));
        assert!(text.contains("2 entries"));
        // The first row's bar reaches the full width of the bar column.
        let first_bar = text
            .lines()
            .find(|line| line.contains("workspaces"))
            .unwrap();
        assert!(
            first_bar.contains("█"),
            "the largest row must draw a full block bar: {first_bar}"
        );
    }

    #[test]
    fn the_text_map_adapts_to_narrow_terminals() {
        let rows: Vec<TextRow> = (0..6)
            .map(|index| TextRow {
                id: index,
                name: format!("directory-with-a-long-name-{index}"),
                size_bytes: 1 << (20 - index),
                files: 10 * index,
                note: None,
                muted: true,
            })
            .collect();
        for width in [24_usize, 40, 80, 120] {
            let text = render_text(&rows, width);
            for line in text.lines() {
                assert!(
                    line.chars().count() <= width + 24,
                    "line exceeds a {width}-column budget: {line}"
                );
            }
            assert!(text.contains("6 entries"), "summary lost at width {width}");
        }
    }

    #[test]
    fn byte_sizes_read_like_a_human_wrote_them() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
        assert_eq!(human_bytes(300 * 1024 * 1024 * 1024), "300 GiB");
    }
}
