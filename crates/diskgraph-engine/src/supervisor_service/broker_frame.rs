use std::io;

/// 参数：kind/length为固定业务通道种类与正文长度；返回：有限帧头或明确拒绝。
pub(super) fn header(kind: u8, length: usize) -> io::Result<[u8; 5]> {
    if !matches!(kind, 1..=3) || length == 0 || length > 4096 || (kind == 3 && length != 4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid bounded broker frame",
        ));
    }
    let mut bytes = [0; 5];
    bytes[0] = kind;
    bytes[1..].copy_from_slice(&(length as u32).to_be_bytes());
    Ok(bytes)
}

/// 参数：bytes为精确五字节帧头；返回：种类和有界正文长度，正文分配前必须验证。
pub(super) fn decode(bytes: [u8; 5]) -> io::Result<(u8, usize)> {
    let length = u32::from_be_bytes(bytes[1..].try_into().expect("fixed header")) as usize;
    header(bytes[0], length)?;
    Ok((bytes[0], length))
}
