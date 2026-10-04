use diskgraph_core::{UnixFileObservation, UnixObservationGap};
/// 已持久 Unix 强身份观察或固定缺失原因；来源：原生 Rust D42 / FS-02。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredUnixObservation {
    pub observation: Option<UnixFileObservation>,
    pub gap: Option<UnixObservationGap>,
}
