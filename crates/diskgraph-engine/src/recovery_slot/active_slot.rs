use super::SlotError;
use std::fs::File;
use std::time::Instant;
/// 原监督容量锁的活动能力；来源：PF-06，无 Java 对等对象。
/// Drop 只关闭原锁，不清除 ACTIVE；只有原绑定恢复对象可在实际回收后写 CLEAN，公开 API 不提供解锁。
#[must_use = "retain original active capacity; drop is not confirmed cleanup"]
pub struct ActiveSlot {
    pub(super) file: File,
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(super) retiring: bool,
}
impl ActiveSlot {
    /// 参数：原期限；返回：在原锁内同步回读 CLEAN 后显式释放锁；仅原绑定 owner 实际回收后调用。
    /// 解锁成功为最终释放点，调用方必须立即消费原槽，不再执行可重试的记录写入。
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(crate) fn confirm_original_cleanup(&mut self, deadline: Instant) -> Result<(), SlotError> {
        super::slot_reservation::check(deadline)?;
        let record = super::slot_reservation::read_record(&mut self.file)?;
        if record != *b"DGSL01A\n" && !(self.retiring && record == *b"DGSL01C\n") {
            return Err(SlotError::InvalidRecord);
        }
        self.retiring = true;
        super::slot_reservation::write_record(&mut self.file, b"DGSL01C\n", deadline)?;
        // 原资源已清理且 CLEAN 已同步回读；显式释放同一锁，不依赖继承句柄全部关闭。
        // 成功后不再检查 deadline，避免已释放容量又返回可重试错误。
        self.file.unlock().map_err(SlotError::Io)
    }

    /// 参数：deadline 为原检查期限；返回：原 held 记录仍 ACTIVE，否则拒绝。
    /// 验证记录不表示原资源已回收，亦不授予释放容量能力。
    pub fn verify_active(&mut self, deadline: Instant) -> Result<(), SlotError> {
        super::slot_reservation::check(deadline)?;
        if super::slot_reservation::read_record(&mut self.file)? != *b"DGSL01A\n" {
            return Err(SlotError::InvalidRecord);
        }
        super::slot_reservation::check(deadline)
    }
}
