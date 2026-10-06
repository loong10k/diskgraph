use super::{ActiveSlot, SlotError};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::Instant;
/// 工作出生前持有原 native 文件锁的预留；来源：PF-06，无 Java 对等对象。
/// 调用方必须先认证稳定命名空间和唯一未克隆 File；本类型不接受路径或授予文件权限。
#[must_use = "retain original slot through explicit prebirth abort or activation"]
pub struct SlotReservation {
    file: File,
}
impl SlotReservation {
    /// 参数：唯一可信读写文件和原期限；返回：已同步预留或原拒绝，绝不覆盖未确认记录。
    pub fn acquire(mut file: File, deadline: Instant) -> Result<Self, SlotError> {
        check(deadline)?;
        let metadata = file.metadata()?;
        check(deadline)?;
        if !metadata.is_file() {
            return Err(SlotError::Unsupported);
        }
        // 不能用进程内 Mutex 或文件存在性代替真实跨进程原生独占锁。
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => SlotError::Busy,
            std::fs::TryLockError::Error(error) => SlotError::Io(error),
        })?;
        check(deadline)?;
        match file.metadata()?.len() {
            0 => {}
            8 => match read_record(&mut file)? {
                bytes if bytes == *b"DGSL01C\n" => {}
                bytes if bytes == *b"DGSL01R\n" || bytes == *b"DGSL01A\n" => {
                    return Err(SlotError::Unconfirmed);
                }
                _ => return Err(SlotError::InvalidRecord),
            },
            _ => return Err(SlotError::InvalidRecord),
        }
        check(deadline)?;
        // 必须在任何工作出生前同步 RESERVED；掉电/失败不能自动改回 CLEAN。
        write_record(&mut file, b"DGSL01R\n", deadline)?;
        Ok(Self { file })
    }
    /// 参数：原期限；返回：出生前显式取消并持久清除预留。活动类型没有此接口。
    pub fn abort_before_birth(mut self, deadline: Instant) -> Result<(), SlotError> {
        check(deadline)?;
        if read_record(&mut self.file)? != *b"DGSL01R\n" {
            return Err(SlotError::InvalidRecord);
        }
        write_record(&mut self.file, b"DGSL01C\n", deadline)?;
        self.file.unlock()?;
        check(deadline)
    }
    /// 参数：原期限；返回：同步 ACTIVE 后的原锁持有者；失败保留未确认记录。
    pub fn activate(mut self, deadline: Instant) -> Result<ActiveSlot, SlotError> {
        check(deadline)?;
        if read_record(&mut self.file)? != *b"DGSL01R\n" {
            return Err(SlotError::InvalidRecord);
        }
        write_record(&mut self.file, b"DGSL01A\n", deadline)?;
        Ok(ActiveSlot {
            file: self.file,
            #[cfg(any(target_os = "linux", target_os = "macos", windows))]
            retiring: false,
        })
    }
}
/// 参数：deadline 为原绝对期限；返回：尚未耗尽时成功，否则拒绝且不刷新预算。
pub(super) fn check(deadline: Instant) -> Result<(), SlotError> {
    if Instant::now() >= deadline {
        Err(SlotError::Deadline)
    } else {
        Ok(())
    }
}
/// 参数：file 为原独占锁持有的文件；返回：精确八字节状态，长度或读取异常原样拒绝。
pub(super) fn read_record(file: &mut File) -> Result<[u8; 8], SlotError> {
    if file.metadata()?.len() != 8 {
        return Err(SlotError::InvalidRecord);
    }
    let mut bytes = [0; 8];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
/// 参数：原 held 文件、精确状态和原期限；返回：同步回读确认，失败保留原锁与原异常。
pub(super) fn write_record(
    file: &mut File,
    bytes: &[u8; 8],
    deadline: Instant,
) -> Result<(), SlotError> {
    check(deadline)?;
    file.seek(SeekFrom::Start(0))?;
    check(deadline)?;
    file.write_all(bytes)?;
    check(deadline)?;
    file.sync_all()?;
    check(deadline)?;
    // 同一 held 文件回读必须精确一致，追加模式或异常写入不能返回有效出生能力。
    if read_record(file)? != *bytes {
        return Err(SlotError::InvalidRecord);
    }
    check(deadline)
}
