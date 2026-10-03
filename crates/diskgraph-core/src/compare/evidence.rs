/// 比较实际达到的证据深度，元数据一致不等于字节一致；来源：DiskGraph 原生 Rust compare::Evidence。
/// How thoroughly two entries were compared.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    /// The path exists on one side only. No test ran.
    Presence,
    /// Size and modification time were compared; contents were not read.
    Metadata,
    /// Contents were read and compared byte for byte.
    Content,
}
