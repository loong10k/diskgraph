//! 原 tui::tests 的行为回归；仅按真实子模块调整显式导入。
use crate::cli_engine_host::CliTestEngine;

use super::palette::{color_from, fit, mix_toward};
use super::{
    Browser, Entry, Layer, PAGE_SIZE, Pseudonyms, draw, draw_authorized_frame, layer_from_nodes,
    load_layer,
};
use crate::tui_frame_reader::TuiFrameReader;
use crate::tui_request::TuiRequest;
use diskgraph_core::PrincipalId;
use diskgraph_engine::EngineError;
use ratatui::Terminal;
use ratatui::style::Color;

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
fn nested_frame_limits_queries_and_keeps_navigation_visible() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    for index in 0..6 {
        let child = root.join(format!("directory-{index}"));
        std::fs::create_dir(&child).unwrap();
        for file in 0..10 {
            std::fs::write(child.join(format!("file-{file}")), b"x").unwrap();
        }
    }
    let engine = CliTestEngine::open(diskgraph_engine::EngineConfig {
        data_dir: directory.path().join("data"),
        ..Default::default()
    })
    .unwrap();
    let principal = PrincipalId::new("tui-test").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &policy).unwrap();
    engine.run_job(&job.job_id, "tui-test").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    engine
        .with_authorized_revision_reader(
            &revision,
            &principal,
            &policy,
            1000,
            |reader, snapshot, deadline| {
                let node = reader.root_node(snapshot)?.unwrap();
                let children = reader.children(snapshot, node.id, 0, 6)?;
                let browser = Browser::new(&revision, layer_from_nodes(node, children, 0, false));
                let mut reads = TuiFrameReader::new(
                    reader,
                    snapshot,
                    deadline,
                    7,
                    TuiFrameReader::display_bytes(browser.current()),
                );
                for child in &browser.current().children[..4] {
                    let page = reads.load_layer(child.id, PAGE_SIZE).unwrap();
                    assert_eq!(page.children.len(), 10);
                }
                assert!(
                    reads
                        .load_layer(browser.current().children[4].id, PAGE_SIZE)
                        .is_none()
                );
                assert_eq!(reads.truncation_reason, Some("query_budget"));
                let backend = ratatui::backend::TestBackend::new(160, 30);
                let mut terminal = Terminal::new(backend).unwrap();
                terminal
                    .draw(|frame| draw(frame, &browser, &mut reads))
                    .unwrap();
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(text.contains("nested view truncated: query_budget"));
                assert!(
                    text.contains("directory-"),
                    "parent blocks remain inspectable"
                );
                let mut expired =
                    TuiFrameReader::new(reader, snapshot, std::time::Instant::now(), 7, 0);
                assert!(expired.load_layer(1, PAGE_SIZE).is_none());
                assert_eq!(expired.truncation_reason, Some("deadline"));
                let mut no_rows = TuiFrameReader::new(reader, snapshot, deadline, 2048, 0);
                assert!(no_rows.load_layer(1, PAGE_SIZE).is_none());
                assert_eq!(no_rows.truncation_reason, Some("node_budget"));
                let mut no_bytes = TuiFrameReader::new(reader, snapshot, deadline, 7, 256 * 1024);
                assert!(
                    no_bytes
                        .load_layer(browser.current().children[0].id, PAGE_SIZE)
                        .is_none()
                );
                assert_eq!(no_bytes.truncation_reason, Some("byte_budget"));
                let mut wide_page = TuiFrameReader::new(reader, snapshot, deadline, 7, 0);
                assert!(
                    wide_page
                        .load_layer(browser.current().parent_id, 2)
                        .is_none(),
                    "partial nested children must not be scaled to fill the parent's full size"
                );
                assert_eq!(wide_page.truncation_reason, Some("nested_page"));
                Ok(())
            },
        )
        .unwrap();

    let request = TuiRequest {
        engine: &engine,
        revision: &revision,
        principal: &principal,
        authorizer: &policy,
    };
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&revision, cached.clone());
    let mut broken_terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 30)).unwrap();
    // 仅记录真实失败阶段；不延长期限、不消费读取错误，也不追加授权或 SQL 调用。
    let missing_started = std::time::Instant::now();
    let paint_entered = std::cell::Cell::new(None);
    let missing_truncation = std::cell::Cell::new(None);
    let paint_returned = std::cell::Cell::new(None);
    let missing = draw_authorized_frame(&mut broken_terminal, &request, &cached, |frame, reads| {
        paint_entered.set(Some(missing_started.elapsed()));
        assert!(reads.load_layer(999999, PAGE_SIZE).is_none());
        missing_truncation.set(Some(reads.truncation_reason));
        draw(frame, &browser, reads);
        paint_returned.set(Some(missing_started.elapsed()));
    });
    if !matches!(
        &missing,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::NotFound
        ))
    ) {
        eprintln!(
            "TUI_MISSING_FRAME result={missing:?} elapsed={:?} paint_entered={:?} missing_truncation={:?} paint_returned={:?}",
            missing_started.elapsed(),
            paint_entered.get(),
            missing_truncation.get(),
            paint_returned.get()
        );
    }
    assert!(matches!(
        missing,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::NotFound
        ))
    ));
    let output: String = broken_terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        output.trim().is_empty(),
        "non-budget failures must not commit a misleading partial frame"
    );
    let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 30)).unwrap();
    let denied = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
        draw(frame, &browser, reads);
        let mut control = engine.control_store().unwrap();
        control
            .revoke_grant(
                &principal,
                &diskgraph_core::Permission::MetadataRead,
                &scope,
            )
            .unwrap();
        control
            .revoke_grant(
                &principal,
                &diskgraph_core::Permission::MetadataRead,
                &diskgraph_engine::admin_scope(),
            )
            .unwrap();
    });
    assert!(matches!(
        denied,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::PermissionDenied
        ))
    ));
    let output: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        output.trim().is_empty(),
        "a grant revoked during paint must not commit sensitive frame data"
    );
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
