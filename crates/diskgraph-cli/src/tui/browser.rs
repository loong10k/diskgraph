//! 唯一现有 TUI 浏览状态及选择、排序、分页行为。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::{Entry, Layer, PAGE_SIZE, Pseudonyms};

/// 保存当前 revision、目录路径、选择身份、排序和显示阈值。
/// 来源：DiskGraph 原生 Rust tui::Browser；无 Java 对应对象。
pub struct Browser {
    pub revision: String,
    pub trail: Vec<Layer>,
    pub selected: usize,
    /// 当前选择的节点身份；递归层拥有自己的列表，不能用父层索引代替节点 ID。
    pub selected_id: Option<u64>,
    pub sort_by_size: bool,
    pub status: String,
    /// 隐藏占父层总大小低于该比例的子节点，避免大量细碎块淹没主要目录。
    pub min_share: f64,
    /// --anonymize 的标签规则由所有页面和下钻层共享，保持同一编号。
    pub pseudonyms: Option<Pseudonyms>,
}

impl Browser {
    /// 建立始终含根层的浏览状态。
    /// 参数：revision 为实际版本，root 为获准导航层。返回：默认大小排序和阈值的状态。
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

    /// 可选启用匿名展示并为缓存根层替换名称。
    /// 参数：anonymize 指定是否匿名化。返回：同一状态，不改变大小和结构。
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

    /// 获取当前屏幕所在目录层。
    /// 参数：无。返回：导航路径末尾的现有目录层。
    pub fn current(&self) -> &Layer {
        self.trail.last().expect("the trail always has a root")
    }

    /// 获取当前可见阈值以上的子节点。
    /// 参数：无。返回：借用的子节点列表，不改变原索引页。
    pub fn visible(&self) -> Vec<&Entry> {
        let layer = self.current();
        let floor = (layer.total_bytes as f64 * self.min_share) as u64;
        layer
            .children
            .iter()
            .filter(|entry| entry.size_bytes >= floor)
            .collect()
    }

    /// 计算当前层因阈值隐藏的子节点数。
    /// 参数：无。返回：页内隐藏数量。
    pub fn hidden(&self) -> usize {
        self.current().children.len() - self.visible().len()
    }

    /// 按当前页排序模式排列借用节点，稳定保留已有页内语义。
    /// 参数：无。返回：可见节点列表；大小相同时按名称排列。
    pub(super) fn ordered(&self) -> Vec<&Entry> {
        let mut entries = self.visible();
        if self.sort_by_size {
            entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.name.cmp(&b.name)));
        } else {
            entries.sort_by(|a, b| a.name.cmp(&b.name));
        }
        entries
    }

    /// 在当前可见子节点中移动选择，空页保持不选择。
    /// 参数：delta 为相对方向和步数。返回：无；保存对应真实节点 ID。
    pub fn move_selection(&mut self, delta: isize) {
        let count = self.visible().len();
        if count == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, count as isize - 1) as usize;
        self.selected_id = self.ordered().get(self.selected).map(|entry| entry.id);
    }

    /// 计算显式前后页位置。
    /// 参数：direction 为前后方向。返回：合法偏移或无可达页面。
    pub fn page_target(&self, direction: isize) -> Option<u64> {
        let layer = self.current();
        match direction {
            1 if layer.has_more => Some(layer.offset.saturating_add(PAGE_SIZE as u64)),
            -1 if layer.offset > 0 => Some(layer.offset.saturating_sub(PAGE_SIZE as u64)),
            _ => None,
        }
    }

    /// 替换当前目录页并复位到首个子节点。
    /// 参数：layer 为已完成授权的同目录页面。返回：无，父路径层数不变。
    pub fn replace_current_page(&mut self, layer: Layer) {
        let selected_id = layer.children.first().map(|entry| entry.id);
        *self.trail.last_mut().expect("the trail always has a root") = layer;
        self.selected = 0;
        self.selected_id = selected_id;
    }

    /// 切换页内大小或名称排序，并复位选择。
    /// 参数：无。返回：无，保留该页全部节点。
    pub fn toggle_sort(&mut self) {
        self.sort_by_size = !self.sort_by_size;
        self.selected = 0;
        self.selected_id = self.ordered().first().map(|entry| entry.id);
    }

    /// 循环可见比例：全部、0.5%、2%、8%。
    /// 参数：无。返回：无，同步更新当前选择。
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
