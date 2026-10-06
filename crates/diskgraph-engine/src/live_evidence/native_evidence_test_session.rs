use super::{EvidenceProbeSession, ProbeLimits};
use crate::{ProbeHost, ProbeRecovery};
use std::ops::{Deref, DerefMut};

/// Windows 真实采样测试会话及独立恢复责任；来源：Rust PF-06，无 Java 对等对象。
/// 不修改生产会话或兼容 API；使用公开宿主绑定入口取得原会话。
pub(super) struct NativeEvidenceTestSession {
    session: Option<EvidenceProbeSession>,
    recovery: ProbeRecovery,
}

impl NativeEvidenceTestSession {
    /// 参数：limits 为原测试预算；返回：实际产品会话与外部恢复责任或原诊断。
    pub(super) fn new(limits: &ProbeLimits) -> Result<Self, String> {
        let (host, recovery) = ProbeHost::new(1).map_err(|error| error.to_string())?;
        let session = EvidenceProbeSession::new_with_probe_host(limits, &host)?;
        Ok(Self {
            session: Some(session),
            recovery,
        })
    }
}

impl Deref for NativeEvidenceTestSession {
    type Target = EvidenceProbeSession;

    fn deref(&self) -> &Self::Target {
        self.session
            .as_ref()
            .expect("original test session is live")
    }
}

impl DerefMut for NativeEvidenceTestSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.session
            .as_mut()
            .expect("original test session is live")
    }
}

impl Drop for NativeEvidenceTestSession {
    fn drop(&mut self) {
        // 先释放原采样会话，再实际恢复同一资源池；该责任在产品 catch 外存活。
        drop(self.session.take());
        let mut reported = false;
        loop {
            match self.recovery.drain() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("native evidence test recovery retains original owner: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            // 测试宿主兼容等待，不证明有限产品 shutdown。
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
