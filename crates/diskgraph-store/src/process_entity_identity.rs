use diskgraph_core::{ProcessStartupIdentity, ServerId};
use serde::Deserialize;
/// 单个已观察进程的严格实体身份映射；来源：原生 Rust D42 / EV-06，不含程序标签。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessEntityIdentity {
    pub(crate) server_id: ServerId,
    pub(crate) startup: ProcessStartupIdentity,
}
