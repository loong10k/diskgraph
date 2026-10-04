//! 终端生命周期及用户事件协调，复用既有浏览状态与请求。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::navigation::{load_layer_anon, load_layer_page_anon};
use super::{Browser, draw, draw_authorized_frame, load_layer};
use crate::tui_request::TuiRequest;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use diskgraph_core::{Authorizer, PrincipalId};
use diskgraph_engine::{Engine, EngineError};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::Stdout;

/// 运行交互浏览直到退出，向 CLI 原样返回中止它的错误。
/// 参数：engine/revision/主体/授权器为同一请求，anonymize 控制显示标签。
/// 返回：正常退出或原授权、存储、终端错误；不新增引擎或后台线程。
pub fn run(
    engine: &Engine,
    revision: &str,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    anonymize: bool,
) -> Result<(), EngineError> {
    let request = TuiRequest {
        engine,
        revision,
        principal,
        authorizer,
    };
    let root = load_layer(&request, 1)?;
    let mut browser = Browser::new(revision, root).with_pseudonyms(anonymize);
    let mut terminal = setup().map_err(|error| {
        EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
    })?;
    let outcome = event_loop(&mut terminal, &request, &mut browser);
    restore(&mut terminal).ok();
    outcome
}

/// 原终端后端类型别名；来源：DiskGraph 原生 Rust tui::Term，无 Java 对应对象。
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
    request: &TuiRequest<'_>,
    browser: &mut Browser,
) -> Result<(), EngineError> {
    loop {
        draw_authorized_frame(terminal, request, browser.current(), |frame, reads| {
            draw(frame, browser, reads);
        })?;
        // 空闲时仅复核授权，不重复读取/绘制整个地图；撤权不等待下一次按键。
        while !event::poll(std::time::Duration::from_millis(250)).map_err(io_to_engine)? {
            request.engine.authorize_revision(
                None,
                request.revision,
                request.principal,
                request.authorizer,
            )?;
        }
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
                        request,
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
                        request,
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
                        let layer =
                            load_layer_anon(request, entry.id, browser.pseudonyms.as_ref(), false)?;
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
