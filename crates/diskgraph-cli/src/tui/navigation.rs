//! 完整目录页的有界授权读取与匿名化。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::{Layer, Pseudonyms};
use crate::tui_frame_reader::TuiFrameReader;
use crate::tui_request::{TUI_DISPLAY_BYTES, TuiRequest};
use diskgraph_engine::{EngineError, RevisionDisplayCompletion};

/// 读取目录首个完整页面并按同一匿名规则替换名称。
/// 参数：request 为真实身份，parent_id 为父节点，pseudonyms/is_root 指定标签。
/// 返回：已完成末段授权的导航层或原读取错误。
pub(super) fn load_layer_anon(
    request: &TuiRequest<'_>,
    parent_id: u64,
    pseudonyms: Option<&Pseudonyms>,
    is_root: bool,
) -> Result<Layer, EngineError> {
    load_layer_page_anon(request, parent_id, 0, pseudonyms, is_root)
}

/// 读取指定完整页面后匿名化，不改变页顺序、大小或结构。
/// 参数：offset 保留显式分页位置，其余参数为同一身份和标签规则。
/// 返回：通过授权和原期限的导航层或错误。
pub(super) fn load_layer_page_anon(
    request: &TuiRequest<'_>,
    parent_id: u64,
    offset: u64,
    pseudonyms: Option<&Pseudonyms>,
    is_root: bool,
) -> Result<Layer, EngineError> {
    let mut layer = load_layer_page(request, parent_id, offset)?;
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

/// 每次导航读取的最大子节点数。
pub(super) const PAGE_SIZE: usize = 512;

/// 读取 revision 中指定目录的首个完整页面，未索引和拒权保留原错误。
/// 参数：request 包含真实主体和授权器，parent_id 为 revision 内父节点。
/// 返回：末段授权及原读取期限通过的导航层；不会交付部分页。
pub fn load_layer(request: &TuiRequest<'_>, parent_id: u64) -> Result<Layer, EngineError> {
    load_layer_page(request, parent_id, 0)
}

fn load_layer_page(
    request: &TuiRequest<'_>,
    parent_id: u64,
    offset: u64,
) -> Result<Layer, EngineError> {
    let mut prepared = None;
    request.engine.with_authorized_revision_display_reader(
        request.revision,
        request.principal,
        request.authorizer,
        1000,
        |reader, snapshot, deadline| {
            let mut budget = TuiRequest::read_budget(deadline);
            let layer = TuiRequest::read_layer(
                reader,
                snapshot,
                parent_id,
                offset,
                PAGE_SIZE,
                &mut budget,
            )?;
            if TuiFrameReader::display_bytes(&layer) > TUI_DISPLAY_BYTES {
                return Err(diskgraph_store::StoreError::BudgetExceeded.into());
            }
            // 导航只允许完整页；末段授权及原读取期限通过前不向浏览器交付准备结果。
            prepared = Some(layer);
            Ok(RevisionDisplayCompletion::Complete)
        },
    )?;
    prepared.ok_or_else(|| {
        diskgraph_store::StoreError::InvalidGraph(
            "authorized navigation did not prepare a layer".into(),
        )
        .into()
    })
}
