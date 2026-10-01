//! The terminal surface: an interactive treemap over a published revision.
//!
//! Where `diskgraph tree` hands the whole tree over in one go, this walks
//! it: every directory you enter pulls only that directory's children, so a
//! four-million-node home index opens instantly and memory stays flat. The
//! layout and the palette are the same ones the HTML page and the agent
//! text format use, so all three agree on what the map means.

use std::io::Stdout;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use diskgraph_core::treemap::{self, Rect as MapRect, Weighted};
use diskgraph_engine::{Engine, EngineError};

use crate::html::{PALETTE, color_for};

/// One directory level, loaded on demand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub parent_id: u64,
    pub offset: u64,
    pub has_more: bool,
    pub name: String,
    pub total_bytes: u64,
    pub total_files: u64,
    pub unreadable: bool,
    pub children: Vec<Entry>,
}

/// Loaded with `--anonymize`: the root reads as the caller's label and every
/// child becomes a stable pseudonym based on its revision-local node ID.
/// This avoids retaining every visited node when paging a wide directory.
/// Sizes, structure and categories stay exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pseudonyms;

impl Pseudonyms {
    pub fn new() -> Self {
        Self
    }

    /// The root's share-safe label.
    pub fn root(&self) -> &'static str {
        "home"
    }

    fn label_for(&self, node_id: u64) -> String {
        format!("dir-{node_id:02}")
    }
}

fn load_layer_anon(
    engine: &Engine,
    revision: &str,
    parent_id: u64,
    pseudonyms: Option<&Pseudonyms>,
    is_root: bool,
) -> Result<Layer, EngineError> {
    load_layer_page_anon(engine, revision, parent_id, 0, pseudonyms, is_root)
}

fn load_layer_page_anon(
    engine: &Engine,
    revision: &str,
    parent_id: u64,
    offset: u64,
    pseudonyms: Option<&Pseudonyms>,
    is_root: bool,
) -> Result<Layer, EngineError> {
    let mut layer = load_layer_page(engine, revision, parent_id, offset)?;
    if let Some(state) = pseudonyms {
        if is_root {
            layer.name = state.root().to_owned();
        }
        for child in &mut layer.children {
            child.name = state.label_for(child.id);
        }
    }
    Ok(layer)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub size_bytes: u64,
    pub files: u64,
    pub kind: String,
    pub category: Option<String>,
    pub has_children: bool,
    pub read_error: bool,
}

/// What the browser surface calls a revision tree, loaded one level at a
/// time. A scope without an index is a clean "not indexed" report, not a
/// panic.
const PAGE_SIZE: usize = 512;

pub fn load_layer(engine: &Engine, revision: &str, parent_id: u64) -> Result<Layer, EngineError> {
    load_layer_page(engine, revision, parent_id, 0)
}

fn load_layer_page(
    engine: &Engine,
    revision: &str,
    parent_id: u64,
    offset: u64,
) -> Result<Layer, EngineError> {
    let (node, children, has_more) =
        engine.revision_layer_page(revision, parent_id, offset, PAGE_SIZE)?;
    Ok(Layer {
        parent_id,
        offset,
        has_more,
        name: node.name.clone(),
        total_bytes: node.subtree_bytes,
        total_files: node.files,
        unreadable: node.read_error,
        children: children
            .iter()
            .map(|child| Entry {
                id: child.id,
                name: child.name.clone(),
                size_bytes: child.subtree_bytes,
                files: child.files,
                kind: format!("{:?}", child.kind).to_lowercase(),
                category: child.category_hint.clone(),
                has_children: child.directories > 0,
                read_error: child.read_error,
            })
            .collect(),
    })
}

/// The interactive state: where we are, what is selected, how the map is
/// sorted.
pub struct Browser {
    pub revision: String,
    pub trail: Vec<Layer>,
    pub selected: usize,
    /// The selected entry's identity. A recursive level has its own list, so
    /// the index says nothing there; the id does.
    pub selected_id: Option<u64>,
    pub sort_by_size: bool,
    pub status: String,
    /// Children below this share of the layer total are left out of the map:
    /// on a real workspace the top level is one huge project and forty
    /// slivers, and forty slivers are not a picture.
    pub min_share: f64,
    /// Set with `--anonymize`: shared across every layer load so a descended
    /// session keeps one stable numbering.
    pub pseudonyms: Option<Pseudonyms>,
}

impl Browser {
    pub fn new(revision: &str, root: Layer) -> Self {
        Self {
            selected_id: root.children.first().map(|entry| entry.id),
            revision: revision.to_owned(),
            trail: vec![root],
            selected: 0,
            sort_by_size: true,
            status: "↑↓ move · enter descend · n/p pages · backspace up · s sort page · m threshold · q quit"
                .to_owned(),
            min_share: 0.005,
            pseudonyms: None,
        }
    }

    pub fn with_pseudonyms(mut self, anonymize: bool) -> Self {
        if anonymize {
            let state = Pseudonyms::new();
            let root = self.trail.last_mut().expect("the trail always has a root");
            root.name = state.root().to_owned();
            for child in &mut root.children {
                child.name = state.label_for(child.id);
            }
            self.pseudonyms = Some(state);
        }
        self
    }

    /// The layer currently on screen, ordered the way the user asked.
    pub fn current(&self) -> &Layer {
        self.trail.last().expect("the trail always has a root")
    }

    /// The layer's children above the visibility threshold.
    pub fn visible(&self) -> Vec<&Entry> {
        let layer = self.current();
        let floor = (layer.total_bytes as f64 * self.min_share) as u64;
        layer
            .children
            .iter()
            .filter(|entry| entry.size_bytes >= floor)
            .collect()
    }

    /// How many children the threshold leaves out of this layer.
    pub fn hidden(&self) -> usize {
        self.current().children.len() - self.visible().len()
    }

    fn ordered(&self) -> Vec<&Entry> {
        let mut entries = self.visible();
        if self.sort_by_size {
            entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.name.cmp(&b.name)));
        } else {
            entries.sort_by(|a, b| a.name.cmp(&b.name));
        }
        entries
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.visible().len();
        if count == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, count as isize - 1) as usize;
        self.selected_id = self.ordered().get(self.selected).map(|entry| entry.id);
    }

    pub fn page_target(&self, direction: isize) -> Option<u64> {
        let layer = self.current();
        match direction {
            1 if layer.has_more => Some(layer.offset.saturating_add(PAGE_SIZE as u64)),
            -1 if layer.offset > 0 => Some(layer.offset.saturating_sub(PAGE_SIZE as u64)),
            _ => None,
        }
    }

    pub fn replace_current_page(&mut self, layer: Layer) {
        let selected_id = layer.children.first().map(|entry| entry.id);
        *self.trail.last_mut().expect("the trail always has a root") = layer;
        self.selected = 0;
        self.selected_id = selected_id;
    }

    pub fn toggle_sort(&mut self) {
        self.sort_by_size = !self.sort_by_size;
        self.selected = 0;
        self.selected_id = self.ordered().first().map(|entry| entry.id);
    }

    /// Cycles the visibility threshold: everything, then 0.5%, 2%, 8%.
    pub fn cycle_threshold(&mut self) {
        self.min_share = match self.min_share {
            share if share <= 0.0001 => 0.005,
            share if share < 0.01 => 0.02,
            share if share < 0.06 => 0.08,
            _ => 0.0,
        };
        self.selected = 0;
        self.selected_id = self.ordered().first().map(|entry| entry.id);
    }
}

/// Runs the browser until the user quits. Returns the error that stopped it,
/// so a caller can report a failure the way the rest of the CLI does.
pub fn run(engine: &Engine, revision: &str, anonymize: bool) -> Result<(), EngineError> {
    let root = load_layer(engine, revision, 1)?;
    let mut browser = Browser::new(revision, root).with_pseudonyms(anonymize);
    let mut terminal = setup().map_err(|error| {
        EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
    })?;
    let outcome = event_loop(&mut terminal, engine, &mut browser);
    restore(&mut terminal).ok();
    outcome
}

type Term = Terminal<CrosstermBackend<Stdout>>;

fn setup() -> std::io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore(terminal: &mut Term) -> std::io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}

fn event_loop(
    terminal: &mut Term,
    engine: &Engine,
    browser: &mut Browser,
) -> Result<(), EngineError> {
    loop {
        terminal
            .draw(|frame| draw(frame, browser, engine))
            .map_err(io_to_engine)?;
        let Event::Key(key) = event::read().map_err(io_to_engine)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('s') => browser.toggle_sort(),
            KeyCode::Char('m') => browser.cycle_threshold(),
            KeyCode::Char('n') | KeyCode::PageDown => {
                if let Some(offset) = browser.page_target(1) {
                    let parent_id = browser.current().parent_id;
                    let layer = load_layer_page_anon(
                        engine,
                        &browser.revision,
                        parent_id,
                        offset,
                        browser.pseudonyms.as_ref(),
                        browser.trail.len() == 1,
                    )?;
                    browser.replace_current_page(layer);
                }
            }
            KeyCode::Char('p') | KeyCode::PageUp => {
                if let Some(offset) = browser.page_target(-1) {
                    let parent_id = browser.current().parent_id;
                    let layer = load_layer_page_anon(
                        engine,
                        &browser.revision,
                        parent_id,
                        offset,
                        browser.pseudonyms.as_ref(),
                        browser.trail.len() == 1,
                    )?;
                    browser.replace_current_page(layer);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => browser.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => browser.move_selection(1),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                if browser.trail.len() > 1 {
                    browser.trail.pop();
                    browser.selected = 0;
                    browser.selected_id = browser.ordered().first().map(|entry| entry.id);
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let entries = browser.ordered();
                if let Some(entry) = entries.get(browser.selected) {
                    if entry.has_children {
                        let layer = load_layer_anon(
                            engine,
                            &browser.revision.clone(),
                            entry.id,
                            browser.pseudonyms.as_ref(),
                            false,
                        )?;
                        browser.selected_id = layer.children.first().map(|child| child.id);
                        browser.trail.push(layer);
                        browser.selected = 0;
                    } else {
                        browser.status =
                            format!("{} is a file - nothing to descend into", entry.name);
                    }
                }
            }
            _ => {}
        }
    }
}

fn io_to_engine(error: std::io::Error) -> EngineError {
    EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
}

/// Paints one frame: a status bar, the map, and the selection's details.
fn draw(frame: &mut ratatui::Frame<'_>, browser: &Browser, engine: &Engine) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(rows[1]);

    frame.render_widget(status_line(browser), rows[0]);
    draw_map(frame, browser, engine, columns[0]);
    draw_details(frame, browser, columns[1]);
}

fn status_line<'a>(browser: &'a Browser) -> Paragraph<'a> {
    let layer = browser.current();
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", layer.name),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
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

/// The treemap itself: every child gets an area proportional to its bytes,
/// drawn with the block glyphs that share the other surfaces' palette.
fn draw_map(frame: &mut ratatui::Frame<'_>, browser: &Browser, engine: &Engine, area: Rect) {
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
    draw_children(frame, browser, engine, &entries, inner, 0);
}

/// Paints one directory's children into `area`, and recurses while a child is
/// large enough to be worth splitting - a 270 GiB block with nothing inside it
/// says less than the same block holding its own top children.
fn draw_children(
    frame: &mut ratatui::Frame<'_>,
    browser: &Browser,
    engine: &Engine,
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
        // Brightness carries the share, so the biggest child is the most
        // saturated and the eye finds it before it reads a label.
        let share = entry.size_bytes as f64 / largest as f64;
        let base = color_from(entry.category.as_deref());
        let color = mix_toward(base, Color::Black, (1.0 - share) * 0.45);
        // Below a few columns a name is unreadable noise, so the block keeps
        // its area and its colour and says nothing.
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
            // One extra query per level, not a full-graph load: this is what
            // keeps a four-million-node index instant.
            if let Ok(layer) = load_layer_anon(
                engine,
                &browser.revision.clone(),
                entry.id,
                browser.pseudonyms.as_ref(),
                false,
            ) {
                // One child means one column of the same colour and no
                // information: keep the block solid and let Enter reveal it.
                if layer.children.len() > 1 {
                    let deeper: Vec<&Entry> = layer.children.iter().collect();
                    draw_children(frame, browser, engine, &deeper, cell, depth + 1);
                }
            }
        }
    }
}

/// How many levels the map paints before it leaves a block solid; descending
/// further is what Enter is for.
const MAX_PAINT_DEPTH: usize = 2;

/// Blends a colour toward another one, for the share-to-brightness ramp.
fn mix_toward(from: Color, to: Color, amount: f64) -> Color {
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

/// Truncates a label to the cell, so a narrow block shows a name rather than
/// a wrapped one.
fn fit(label: &str, width: u16) -> String {
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

fn color_from(category: Option<&str>) -> Color {
    let hex = color_for(category);
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x4a5462);
    Color::Rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
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

/// The palette's legend, for surfaces that can spare the room.
#[allow(dead_code, reason = "the legend is shared with documentation tools")]
pub fn legend() -> Vec<(&'static str, &'static str)> {
    PALETTE
        .iter()
        .filter(|(name, _)| !name.is_empty())
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, name: &str, size: u64, files: u64) -> Entry {
        Entry {
            id,
            name: name.into(),
            size_bytes: size,
            files,
            kind: "directory".into(),
            category: Some("code".into()),
            has_children: true,
            read_error: false,
        }
    }

    fn layer() -> Layer {
        Layer {
            parent_id: 1,
            offset: 0,
            has_more: false,
            name: "root".into(),
            total_bytes: 300,
            total_files: 30,
            unreadable: false,
            children: vec![
                entry(1, "workspaces", 200, 20),
                entry(2, "cache", 60, 8),
                entry(3, "media", 40, 2),
            ],
        }
    }

    #[test]
    fn selection_stays_inside_the_layer() {
        let mut browser = Browser::new("rev-1", layer());
        browser.move_selection(-1);
        assert_eq!(browser.selected, 0, "cannot move above the first entry");
        browser.move_selection(99);
        assert_eq!(browser.selected, 2, "cannot move past the last entry");
        browser.move_selection(-1);
        assert_eq!(browser.selected, 1);
    }

    #[test]
    fn wide_directory_can_navigate_bounded_pages_without_losing_parent() {
        let mut first = layer();
        first.parent_id = 17;
        first.has_more = true;
        let mut browser = Browser::new("rev-1", first);
        assert_eq!(browser.page_target(1), Some(PAGE_SIZE as u64));
        assert_eq!(browser.page_target(-1), None);

        let mut second = layer();
        second.parent_id = 17;
        second.offset = PAGE_SIZE as u64;
        second.has_more = false;
        second.children = vec![entry(99, "last", 1, 1)];
        browser.replace_current_page(second);
        assert_eq!(browser.current().parent_id, 17);
        assert_eq!(browser.selected_id, Some(99));
        assert_eq!(browser.page_target(1), None);
        assert_eq!(browser.page_target(-1), Some(0));
    }

    #[test]
    fn anonymized_nodes_keep_their_labels_when_a_page_is_revisited() {
        let pseudonyms = Pseudonyms::new();
        let first = pseudonyms.label_for(11);
        let second = pseudonyms.label_for(12);
        assert_ne!(first, second);
        assert_eq!(pseudonyms.label_for(11), first);
    }

    #[test]
    fn sorting_by_size_and_by_name_reorders_without_losing_entries() {
        let mut browser = Browser::new("rev-1", layer());
        assert_eq!(browser.ordered()[0].name, "workspaces", "largest first");
        browser.toggle_sort();
        let names: Vec<&str> = browser.ordered().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["cache", "media", "workspaces"]);
        assert_eq!(browser.ordered().len(), 3, "sorting never drops an entry");
    }

    #[test]
    fn the_threshold_hides_slivers_and_says_how_many() {
        let mut big = layer();
        big.children.push(entry(4, "a-large-one", 1_000, 10));
        big.total_bytes += 1_000;
        big.total_files += 10;
        let mut browser = Browser::new("rev-1", big);
        // 1,000 out of 1,300 is 77%: visible. The 60-byte cache is 4.6%:
        // visible at the default 0.5% and hidden once the threshold rises.
        assert_eq!(browser.visible().len(), 4);
        browser.min_share = 0.05;
        assert_eq!(browser.visible().len(), 2, "cache and media fall below 5%");
        assert_eq!(browser.hidden(), 2, "the map says what it left out");
        browser.cycle_threshold();
        assert_eq!(browser.min_share, 0.08);
        browser.cycle_threshold();
        assert_eq!(browser.min_share, 0.0, "the cycle returns to showing all");
        assert_eq!(browser.hidden(), 0);
    }

    #[test]
    fn an_empty_layer_selects_nothing_rather_than_panicking() {
        let mut browser = Browser::new(
            "rev-1",
            Layer {
                name: "empty".into(),
                ..Layer::default()
            },
        );
        browser.move_selection(1);
        assert_eq!(browser.selected, 0);
        assert!(browser.ordered().is_empty());
    }

    #[test]
    fn the_browser_always_has_a_root_to_draw() {
        let browser = Browser::new("rev-1", layer());
        assert_eq!(browser.current().name, "root");
        assert_eq!(browser.revision, "rev-1");
    }

    #[test]
    fn anonymize_replaces_every_name_but_keeps_the_numbers() {
        let state = Pseudonyms::new();
        let mut root = layer();
        root.name = state.root().to_owned();
        for child in &mut root.children {
            child.name = state.label_for(child.id);
        }
        assert_eq!(root.name, "home");
        let names: Vec<&str> = root.children.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["dir-01", "dir-02", "dir-03"]);
        assert_eq!(root.children[0].size_bytes, 200, "sizes survive");
        assert_eq!(root.total_files, 30, "totals survive");
        // Revisited pages and later layers map the same ID to the same label.
        assert_eq!(state.label_for(4), "dir-04");
    }

    #[test]
    fn anonymize_off_leaves_names_alone() {
        let browser = Browser::new("rev-1", layer()).with_pseudonyms(false);
        assert!(browser.pseudonyms.is_none());
        assert_eq!(browser.current().children[0].name, "workspaces");
    }

    fn channels(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        }
    }

    #[test]
    fn a_smaller_share_always_renders_darker() {
        let base = color_from(Some("code"));
        let big = mix_toward(base, Color::Black, 0.0);
        let small = mix_toward(base, Color::Black, 0.45);
        let (bigger, dimmer) = (channels(big), channels(small));
        assert!(
            bigger.0 > dimmer.0 && bigger.1 > dimmer.1 && bigger.2 > dimmer.2,
            "share must read as brightness"
        );
    }

    #[test]
    fn labels_are_cut_to_the_cell_rather_than_wrapped() {
        assert_eq!(fit("workspaces 270 MiB", 20), "workspaces 270 MiB");
        assert_eq!(fit("workspaces 270 MiB", 10), "workspace…");
        assert_eq!(fit("x", 4), "x");
        assert_eq!(fit("abcdefgh", 1), "…");
    }

    #[test]
    fn colours_come_from_the_shared_palette() {
        assert_eq!(color_from(Some("code")), Color::Rgb(0x3f, 0x6e, 0xa8));
        assert_eq!(color_from(None), Color::Rgb(0x4a, 0x54, 0x62));
    }
}
