use super::SlotError;
use std::fs::File;
use std::time::Instant;
/// 原监督容量锁的不可退休活动能力；来源：PF-06，无 Java 对等对象。
/// Drop 只关闭原锁，不清除 ACTIVE；原生恢复绑定及安全退休尚未实现。
#[must_use = "retain original active capacity; drop is not confirmed cleanup"]
pub struct ActiveSlot {
    pub(super) file: File,
}
impl ActiveSlot {
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
