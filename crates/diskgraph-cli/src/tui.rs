//! 终端展示入口；真实对象、导航、绘制和事件生命周期由职责模块维护。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

mod authorized_frame;
mod browser;
mod entry;
mod layer;
mod navigation;
mod palette;
mod pseudonyms;
mod renderer;
mod session;
mod treemap;

use authorized_frame::draw_authorized_frame;
pub use browser::Browser;
pub use entry::Entry;
pub use layer::Layer;
pub(crate) use layer::layer_from_navigation_nodes;
use navigation::PAGE_SIZE;
pub use navigation::load_layer;
#[expect(
    unused_imports,
    reason = "保留原 tui::legend 文档工具导出路径，生产终端不绘制图例"
)]
pub use palette::legend;
pub use pseudonyms::Pseudonyms;
use renderer::draw;
pub use session::run;

#[cfg(test)]
pub(crate) use layer::layer_from_nodes;
#[cfg(test)]
#[path = "tui_deadline_frame_tests.rs"]
mod deadline_frame_tests;
#[cfg(test)]
#[path = "tui_deadline_phase_authorizer.rs"]
mod deadline_phase_authorizer;
#[cfg(test)]
#[path = "tui_input_budget_tests.rs"]
mod input_budget_tests;
#[cfg(test)]
#[path = "tui_test_fixture.rs"]
mod test_fixture;
#[cfg(test)]
mod tests;
