use crate::EngineError;
use diskgraph_core::BusinessError;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::time::Instant;

const MAX_COMMAND_BYTES: usize = 1024 * 1024;
const SYSTEM_LIBRARIES: [&[u8]; 5] = [
    b"/usr/lib/libSystem.B.dylib",
    b"/usr/lib/libobjc.A.dylib",
    b"/usr/lib/libiconv.2.dylib",
    b"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation",
    b"/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation",
];

/// 从原镜像句柄核验固定macOS helper加载命令，不执行解析目标或发现用户目录依赖。
/// 来源：Apple SDK mach-o/loader.h 与原生 Rust PF-06；无 Java 对等对象。
/// 受信OS的dyld/system库另行资格验收；本对象不证明代码签名或完整Mach-O有效性。
pub(super) struct MacosLoadPolicy;

impl MacosLoadPolicy {
    /// 参数：image/bytes来自原认证租约，deadline/checkpoint沿原请求；返回：准入或原错误。
    /// 最多分配1MiB命令区；不读取整镜像，不改变共享文件偏移，不授予执行权限。
    pub(super) fn validate(
        image: &File,
        bytes: u64,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        check(deadline, checkpoint)?;
        if bytes < 32 {
            return unsupported();
        }
        let mut header = [0_u8; 32];
        read_exact_at(image, &mut header, 0, deadline, checkpoint)?;
        let cpu = if cfg!(target_arch = "aarch64") {
            0x0100_000c
        } else {
            0x0100_0007
        };
        let count = word(&header, 16)? as usize;
        let size = word(&header, 20)? as usize;
        if word(&header, 0)? != 0xfeed_facf
            || word(&header, 4)? != cpu
            || word(&header, 12)? != 2
            || word(&header, 24)? & 0x180 != 0x80
            || !(2..=4096).contains(&count)
            || size > MAX_COMMAND_BYTES
            || size < count * 8
            || size as u64 > bytes - 32
        {
            return unsupported();
        }
        let mut commands = Vec::new();
        commands
            .try_reserve_exact(size)
            .map_err(|_| BusinessError::BudgetExceeded)?;
        commands.resize(size, 0);
        read_exact_at(image, &mut commands, 32, deadline, checkpoint)?;
        let mut offset = 0_usize;
        let mut linker = false;
        let mut system = false;
        for _ in 0..count {
            check(deadline, checkpoint)?;
            let remaining = commands.get(offset..).ok_or(BusinessError::Unsupported)?;
            let kind = word(remaining, 0)?;
            let length = word(remaining, 4)? as usize;
            if length < 8 || !length.is_multiple_of(8) {
                return unsupported();
            }
            let command = remaining.get(..length).ok_or(BusinessError::Unsupported)?;
            match kind {
                0xe => {
                    if linker || name(command, 12)? != b"/usr/lib/dyld" {
                        return unsupported();
                    }
                    linker = true;
                }
                0xc => {
                    // SDK macOS15+替代编码可在同编号携带weak/reexport等flag，不能按传统name放行。
                    if word(command, 12)? == 0x1a74_1800 {
                        return unsupported();
                    }
                    let library = name(command, 24)?;
                    if !SYSTEM_LIBRARIES.contains(&library) {
                        return unsupported();
                    }
                    system |= library == SYSTEM_LIBRARIES[0];
                }
                // 仅固定SDK结构命令；其他命令包括环境、rpath与替代依赖模式默认拒绝。
                0x19 => {
                    if length < 72 {
                        return unsupported();
                    }
                    let sections = word(command, 64)? as usize;
                    if sections > (MAX_COMMAND_BYTES - 72) / 80 || length != 72 + sections * 80 {
                        return unsupported();
                    }
                }
                0x2 => minimum(command, 24)?,
                0xb => minimum(command, 80)?,
                0x8000_0022 => minimum(command, 48)?,
                0x1b => minimum(command, 24)?,
                0x32 => minimum(command, 24)?,
                0x2a => minimum(command, 16)?,
                0x8000_0028 => minimum(command, 24)?,
                0x24 | 0x26 | 0x29 | 0x1d | 0x8000_0033 | 0x8000_0034 => minimum(command, 16)?,
                _ => return unsupported(),
            }
            offset += length;
        }
        if offset != commands.len() || !linker || !system {
            return unsupported();
        }
        check(deadline, checkpoint)
    }
}

fn name(command: &[u8], minimum_offset: usize) -> Result<&[u8], EngineError> {
    minimum(command, minimum_offset)?;
    let offset = word(command, 8)? as usize;
    if offset < minimum_offset {
        return unsupported();
    }
    let data = command.get(offset..).ok_or(BusinessError::Unsupported)?;
    let end = data
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(BusinessError::Unsupported)?;
    if data[end..].iter().any(|byte| *byte != 0) {
        return unsupported();
    }
    Ok(&data[..end])
}
fn minimum(command: &[u8], size: usize) -> Result<(), EngineError> {
    if command.len() < size {
        return unsupported();
    }
    Ok(())
}
fn word(bytes: &[u8], offset: usize) -> Result<u32, EngineError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(BusinessError::Unsupported)?;
    Ok(u32::from_le_bytes(
        value.try_into().expect("four checked bytes"),
    ))
}
fn read_exact_at(
    image: &File,
    mut target: &mut [u8],
    mut offset: u64,
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    while !target.is_empty() {
        check(deadline, checkpoint)?;
        let count = match image.read_at(target, offset) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        check(deadline, checkpoint)?;
        if count == 0 {
            return Err(BusinessError::Conflict.into());
        }
        offset += count as u64;
        target = &mut target[count..];
    }
    Ok(())
}
fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
fn unsupported<T>() -> Result<T, EngineError> {
    Err(BusinessError::Unsupported.into())
}
