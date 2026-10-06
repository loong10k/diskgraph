use crate::EngineError;
use crate::live_evidence::git_private_directory_owner::GitPrivateDirectoryOwner;
use crate::probe_resource_slot::ProbeResourceSlot;
use crate::probe_session_lease::ProbeSessionLease;
use crate::scan_worker_registry::ScanWorkerRegistry;
use diskgraph_core::BusinessError;
use std::sync::{Arc, Mutex};

/// 固定N个session资源槽，每槽预建一个child registry；来源：PF-06目录与child共同恢复。
pub(crate) struct ProbeResourcePool {
    slots: Mutex<Vec<ProbeResourceSlot>>,
}
impl ProbeResourcePool {
    /// 参数：非零session容量；返回：一次预分配的资源池，不创建进程或目录。
    pub(crate) fn new(capacity: u32) -> Result<Arc<Self>, EngineError> {
        if capacity == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let count = usize::try_from(capacity).map_err(|_| BusinessError::ResourceExhausted)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| BusinessError::ResourceExhausted)?;
        for _ in 0..count {
            slots.push(ProbeResourceSlot {
                inner: ScanWorkerRegistry::new(1)?,
                generation: 0,
                reserved: false,
                session_alive: false,
                directory_borrowed: false,
                draining: false,
                directory: None,
            });
        }
        Ok(Arc::new(Self {
            slots: Mutex::new(slots),
        }))
    }
    /// 参数：原池；返回：唯一session lease，目录创建与child出生前调用。
    pub(crate) fn reserve(self: &Arc<Self>) -> Result<ProbeSessionLease, EngineError> {
        let mut slots = self.slots.lock().map_err(|_| EngineError::Poisoned)?;
        let (index, slot) = slots
            .iter_mut()
            .enumerate()
            .find(|(_, s)| !s.reserved && s.generation < u64::MAX)
            .ok_or(BusinessError::ResourceExhausted)?;
        slot.generation += 1;
        slot.reserved = true;
        slot.session_alive = true;
        Ok(ProbeSessionLease::new(
            Arc::clone(self),
            index,
            slot.generation,
            Arc::clone(&slot.inner),
        ))
    }
    /// 参数：无；返回：实际session占槽，包括借出目录和恢复中的owner。
    pub(crate) fn occupied(&self) -> Result<usize, EngineError> {
        Ok(self
            .slots
            .lock()
            .map_err(|_| EngineError::Poisoned)?
            .iter()
            .filter(|s| s.reserved)
            .count())
    }
    /// 参数：原代次；返回：目录借用资格，同session不允许同时存在第二目录。
    pub(crate) fn borrow_directory(
        &self,
        index: usize,
        generation: u64,
    ) -> Result<(), EngineError> {
        let mut slots = self.slots.lock().map_err(|_| EngineError::Poisoned)?;
        let slot = &mut slots[index];
        if slot.generation != generation
            || !slot.session_alive
            || slot.directory_borrowed
            || slot.directory.is_some()
            || slot.draining
        {
            return Err(BusinessError::ResourceExhausted.into());
        }
        slot.directory_borrowed = true;
        Ok(())
    }
    /// 参数：原代次和同一目录owner；返回：无，交接不分配，不执行OS清理。
    pub(crate) fn return_directory(
        &self,
        index: usize,
        generation: u64,
        owner: Option<GitPrivateDirectoryOwner>,
    ) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slot = &mut slots[index];
        // 唯一binding活着时directory_borrowed阻止任何代次复用。
        assert_eq!(slot.generation, generation);
        slot.directory = owner;
        slot.directory_borrowed = false;
    }
    /// 参数：原槽索引与代次；返回：仍活跃原会话的同一待恢复目录，拒绝回收中或已结束会话。
    /// 仅显式重试调用；锁内移动 owner 并恢复借用标记，OS 清理继续在锁外执行。
    pub(crate) fn reborrow_retained_directory(
        &self,
        index: usize,
        generation: u64,
    ) -> Result<GitPrivateDirectoryOwner, EngineError> {
        let mut slots = self.slots.lock().map_err(|_| EngineError::Poisoned)?;
        let slot = slots
            .get_mut(index)
            .ok_or(BusinessError::ResourceExhausted)?;
        if slot.generation != generation
            || !slot.reserved
            || !slot.session_alive
            || slot.directory_borrowed
            || slot.draining
        {
            return Err(BusinessError::ResourceExhausted.into());
        }
        let owner = slot
            .directory
            .take()
            .ok_or(BusinessError::ResourceExhausted)?;
        slot.directory_borrowed = true;
        Ok(owner)
    }
    /// 参数：原代次、inner是否真实无owner；返回：无，session活着时禁止Recovery释放资源。
    pub(crate) fn finish_session(&self, index: usize, generation: u64, empty: bool) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let slot = &mut slots[index];
        assert_eq!(slot.generation, generation);
        slot.session_alive = false;
        if empty && !slot.directory_borrowed && slot.directory.is_none() && !slot.draining {
            slot.reserved = false;
        }
    }
    /// 参数：无；返回：一轮实际恢复结果，锁外先child wait/Job0再目录删除，失败保留同槽。
    pub(crate) fn drain(&self) -> Result<bool, EngineError> {
        let count = self.slots.lock().map_err(|_| EngineError::Poisoned)?.len();
        let mut first = None;
        for index in 0..count {
            let work = {
                let mut slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let slot = &mut slots[index];
                if !slot.reserved || slot.session_alive || slot.directory_borrowed || slot.draining
                {
                    None
                } else {
                    slot.draining = true;
                    Some((Arc::clone(&slot.inner), slot.directory.take()))
                }
            };
            if let Some((inner, mut directory)) = work {
                let result = inner.drain().and_then(|empty| {
                    if !empty {
                        return Ok(false);
                    }
                    if let Some(owner) = directory.as_mut() {
                        owner
                            .cleanup()
                            .map_err(|error| EngineError::Io(std::io::Error::other(error)))?;
                    }
                    Ok(true)
                });
                let mut slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let slot = &mut slots[index];
                slot.draining = false;
                match result {
                    Ok(true) => {
                        slot.reserved = false;
                    }
                    Ok(false) => {
                        slot.directory = directory;
                    }
                    Err(error) => {
                        slot.directory = directory;
                        if first.is_none() {
                            first = Some(error);
                        }
                    }
                }
            }
        }
        if let Some(error) = first {
            return Err(error);
        }
        Ok(self.occupied()? == 0)
    }
}
