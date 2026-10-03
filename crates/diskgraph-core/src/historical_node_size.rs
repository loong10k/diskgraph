//! 历史尺寸的共同资格判断；来源：DiskGraph 原生 Rust D33 / Q-04。

use crate::DiskNode;

/// 读取有明确观测依据的节点子树大小，不把存储占位数字作为已知事实。
/// 参数：node 为任意扫描、历史或导入节点。
/// 返回：size_known 且未记录 read_error 时的字节数，未知或读取失败为 None。
pub fn observed_node_size(node: &DiskNode) -> Option<u64> {
    (node.size_known && !node.read_error).then_some(node.subtree_bytes)
}

/// 计算同类且两侧尺寸均已观察的路径增长；不推断永久对象身份或内容相等。
/// 参数：before/after 为调用方完成快照兼容性检查并按定位匹配的前后节点。
/// 返回：after 减 before 的有符号差值，保留零和负值；类型替换或未知尺寸为 None。
pub fn comparable_growth_delta(before: &DiskNode, after: &DiskNode) -> Option<i128> {
    if before.kind != after.kind {
        return None;
    }
    Some(i128::from(observed_node_size(after)?) - i128::from(observed_node_size(before)?))
}
