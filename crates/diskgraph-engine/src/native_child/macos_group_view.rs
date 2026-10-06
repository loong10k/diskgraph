use super::ChildError;

/// Darwin 自有私有 session 的完整组视图；未知能力不能被误报为空组或活动组。
/// 来源：原生 Rust libproc PROC_PGRP_ONLY/PROC_PIDTBSDINFO 资格检查。
pub(super) enum MacosGroupView {
    Active,
    AllExited,
    Unknown(ChildError),
}
