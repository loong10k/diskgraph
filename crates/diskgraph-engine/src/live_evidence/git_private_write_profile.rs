use std::cell::RefCell;
use std::time::Instant;

thread_local! {
    static TOTALS: RefCell<([u128; 9], u64, Instant)> = RefCell::new(([0; 9], 0, Instant::now()));
}

/// Windows 测试构建的实际私有写入阶段诊断；来源：原生 Rust 容量回归定位。
/// 显式开启才计时，不改变任何生产预算、系统调用或返回结果。
pub(super) struct GitPrivateWriteProfile {
    last: Instant,
    stages: [u128; 9],
    next_stage: usize,
}

impl GitPrivateWriteProfile {
    /// 参数：无。返回：显式环境开关开启时的阶段计时器，否则不采样。
    pub(super) fn new() -> Option<Self> {
        (std::env::var_os("DG_PRIVATE_WRITE_PROFILE").as_deref() == Some(std::ffi::OsStr::new("1")))
            .then(|| Self {
                last: Instant::now(),
                stages: [0; 9],
                next_stage: 0,
            })
    }

    /// 参数：profile 为同一次写入的诊断槽位。返回：无；累计刚完成的真实阶段耗时。
    pub(super) fn mark(profile: &mut Option<Self>) {
        if let Some(profile) = profile {
            profile.stages[profile.next_stage] = profile.last.elapsed().as_micros();
            profile.next_stage += 1;
            profile.last = Instant::now();
        }
    }
}

impl Drop for GitPrivateWriteProfile {
    fn drop(&mut self) {
        self.stages[self.next_stage] = self.last.elapsed().as_micros();
        TOTALS.with_borrow_mut(|(stages, writes, started)| {
            for (total, elapsed) in stages.iter_mut().zip(self.stages) {
                *total = total.saturating_add(elapsed);
            }
            *writes += 1;
            // 只输出累计量，不追加逐文件日志或改变真实条目数。
            if *writes % 1024 == 0 || self.next_stage != 8 {
                println!("DG_PRIVATE_WRITE_PROFILE writes={writes} elapsed_ms={} stage_us={stages:?} last_stage={}", started.elapsed().as_millis(), self.next_stage);
            }
        });
    }
}
