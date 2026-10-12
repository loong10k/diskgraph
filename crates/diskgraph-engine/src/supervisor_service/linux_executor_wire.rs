use crate::native_deadline::ClockStamp;
use serde::{Deserialize, Serialize};

/// executor请求的严格反序列化字段；来源：PF-06本机出生协议，无Java对等对象。
/// serde同时拒绝重复和未知字段；此对象本身没有身份或部署信任。
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinuxExecutorWire {
    pub(super) schema_version: u16,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) group_count: u32,
    pub(super) groups_sha256: String,
    pub(super) worker_fd: u32,
    pub(super) control_fd: u32,
    pub(super) worker_sha256: String,
    pub(super) worker_bytes: u64,
    pub(super) data_dir: String,
    pub(super) deadline: ClockStamp,
}
