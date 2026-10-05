//! 错误因果回归的请求局部观察点；来源：原生 Rust 持久任务真实执行链。
//! 只同步或借用实际结果，不替换扫描、fence、授权、取消或线程算法。

use crate::EngineError;
use diskgraph_store::{JobRecord, StoreError};
use std::cell::RefCell;

type Phase = (String, Box<dyn FnOnce()>);
type ScanError = (String, Box<dyn FnOnce(&EngineError)>);
type HeartbeatError = (String, Box<dyn FnOnce(&StoreError)>);
type Outcome = (String, Box<dyn FnOnce(&Result<(), EngineError>)>);
type KeeperBeforeCallback = Box<dyn FnOnce(&JobRecord) + Send>;
type KeeperErrorCallback = Box<dyn FnOnce(&JobRecord, &EngineError) + Send>;
type KeeperBefore = (String, KeeperBeforeCallback);
type KeeperError = (String, KeeperErrorCallback);

thread_local! {
    static SPAWN: RefCell<Option<Phase>> = const { RefCell::new(None) };
    static SCAN_ERROR: RefCell<Option<ScanError>> = const { RefCell::new(None) };
    static HEARTBEAT_ERROR: RefCell<Option<HeartbeatError>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Outcome>> = const { RefCell::new(None) };
    static FINAL_SCAN_VALIDATION: RefCell<Option<Phase>> = const { RefCell::new(None) };
    static KEEPER_BEFORE: RefCell<Option<KeeperBefore>> = const { RefCell::new(None) };
    static KEEPER_ERROR: RefCell<Option<KeeperError>> = const { RefCell::new(None) };
}

fn take<T>(slot: &RefCell<Option<(String, T)>>, job: &str) -> Option<T> {
    if slot.borrow().as_ref().is_some_and(|(id, _)| id == job) {
        slot.borrow_mut().take().map(|(_, callback)| callback)
    } else {
        None
    }
}

/// 已认领请求的 keeper 观察配置；来源：原生 Rust 测试局部同步。
/// 配置在调用线程取出后移动进既有 scoped keeper，不能在线程内查另一份 TLS。
pub(super) struct KeeperObservation {
    claimed: JobRecord,
    before: Option<KeeperBeforeCallback>,
    error: Option<KeeperErrorCallback>,
}

impl KeeperObservation {
    /// 参数：无；返回无，在第一次实际 SQL 检查之前且锁外执行一次。
    pub(super) fn before_check(&mut self) {
        if let Some(callback) = self.before.take() {
            callback(&self.claimed);
        }
    }

    /// 参数：error 为已保存且释放控制锁后的真实失败；返回无，在原停止位写入后观察。
    pub(super) fn after_error(&mut self, error: &EngineError) {
        if let Some(callback) = self.error.take() {
            callback(&self.claimed, error);
        }
    }
}

/// 参数：claimed 为实际成功认领记录；返回该任务观察配置，不提供任何执行资格。
pub(super) fn take_keeper(claimed: &JobRecord) -> KeeperObservation {
    KeeperObservation {
        claimed: claimed.clone(),
        before: KEEPER_BEFORE.with(|slot| take(slot, &claimed.job_id)),
        error: KEEPER_ERROR.with(|slot| take(slot, &claimed.job_id)),
    }
}

/// 参数：job/回调绑定请求；返回无，注册真实扫描启动后的观察。
pub(super) fn at_spawn(job: &str, callback: impl FnOnce() + 'static) {
    SPAWN.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：job 为实际扫描任务；返回无，只在真实 ScanHandle 已启动后执行。
pub(super) fn after_spawn(job: &str) {
    SPAWN.with(|slot| {
        if let Some(callback) = take(slot, job) {
            callback();
        }
    });
}

/// 参数：job/回调绑定请求；返回无，注册首个实际 guard 失败观察。
pub(super) fn at_scan_error(job: &str, callback: impl FnOnce(&EngineError) + 'static) {
    SCAN_ERROR.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：job/checked 为真实检查；返回无，不修改失败或停止位。
pub(super) fn scan_checked(job: &str, checked: &Result<(), EngineError>) {
    if let Err(error) = checked {
        SCAN_ERROR.with(|slot| {
            if let Some(callback) = take(slot, job) {
                callback(error);
            }
        });
    }
}

/// 参数：job/回调绑定请求；返回无，注册实际 heartbeat 失败观察。
pub(super) fn at_heartbeat_error(job: &str, callback: impl FnOnce(&StoreError) + 'static) {
    HEARTBEAT_ERROR.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：job/checked 为原 heartbeat 实际返回；返回无，不改变原错误分类。
pub(super) fn heartbeat_checked(job: &str, checked: &Result<(), StoreError>) {
    if let Err(error) = checked {
        HEARTBEAT_ERROR.with(|slot| {
            if let Some(callback) = take(slot, job) {
                callback(error);
            }
        });
    }
}

/// 参数：job/回调绑定请求；返回无，注册实际执行结果产生后的观察。
pub(super) fn at_outcome(job: &str, callback: impl FnOnce(&Result<(), EngineError>) + 'static) {
    OUTCOME.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：job/result 为原执行结果；返回无，在发送 keeper 停止请求之前观察。
pub(super) fn after_execution(job: &str, result: &Result<(), EngineError>) {
    OUTCOME.with(|slot| {
        if let Some(callback) = take(slot, job) {
            callback(result);
        }
    });
}

/// 参数：job/回调绑定请求；返回无，注册最后原生根复核完成后的锁外同步。
pub(super) fn at_final_scan_validation(job: &str, callback: impl FnOnce() + 'static) {
    FINAL_SCAN_VALIDATION.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：job 为真实扫描任务；返回无，不替换验证结果或修改发布许可。
pub(super) fn after_final_scan_validation(job: &str) {
    FINAL_SCAN_VALIDATION.with(|slot| {
        if let Some(callback) = take(slot, job) {
            callback();
        }
    });
}

/// 参数：job/回调绑定请求；返回无，注册真实 keeper SQL 之前的一次同步。
pub(super) fn before_keeper(job: &str, callback: impl FnOnce(&JobRecord) + Send + 'static) {
    KEEPER_BEFORE.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：job/回调绑定请求；返回无，只借用首个实际 keeper 错误。
pub(super) fn at_keeper_error(
    job: &str,
    callback: impl FnOnce(&JobRecord, &EngineError) + Send + 'static,
) {
    KEEPER_ERROR.with(|slot| *slot.borrow_mut() = Some((job.into(), Box::new(callback))));
}

/// 参数：无；返回无，清除本测试尚未消费的观察点，避免测试线程复用时残留。
pub(super) fn clear() {
    SPAWN.with(|slot| slot.borrow_mut().take());
    SCAN_ERROR.with(|slot| slot.borrow_mut().take());
    HEARTBEAT_ERROR.with(|slot| slot.borrow_mut().take());
    OUTCOME.with(|slot| slot.borrow_mut().take());
    FINAL_SCAN_VALIDATION.with(|slot| slot.borrow_mut().take());
    KEEPER_BEFORE.with(|slot| slot.borrow_mut().take());
    KEEPER_ERROR.with(|slot| slot.borrow_mut().take());
}
