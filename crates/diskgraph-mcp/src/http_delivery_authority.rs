//! HTTP 投递时复验原请求到期与实时授权版本。
use crate::McpService;
use crate::request_context::RequestContext;
use std::io::{Error, ErrorKind};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 参数：原共享服务及原deadline；返回：同一窗口内读取的授权版本，无法确认时拒绝。
pub(crate) fn generation_until(service: &McpService, deadline: Instant) -> std::io::Result<u64> {
    while Instant::now() < deadline {
        match service
            .engine()
            .try_control_store()
            .map_err(|_| Error::other("HTTP control unavailable"))?
        {
            Some(store) => {
                return store
                    .authorization_generation_until(deadline)
                    .map_err(|error| {
                        if matches!(error, diskgraph_store::StoreError::BudgetExceeded) {
                            Error::new(ErrorKind::TimedOut, "HTTP authorization deadline exceeded")
                        } else {
                            Error::other("HTTP authorization unavailable")
                        }
                    });
            }
            None => std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(1)),
            ),
        }
    }
    Err(Error::new(
        ErrorKind::TimedOut,
        "HTTP authorization deadline exceeded",
    ))
}

/// 参数：原服务、实际认证上下文、分发前固定版本及原deadline；返回：仍可发送或拒绝原因。
pub(crate) fn authorize_until(
    service: &McpService,
    context: &RequestContext,
    generation: u64,
    deadline: Instant,
) -> std::io::Result<()> {
    let unexpired = || {
        context.expires_at().is_none_or(|expires| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .is_ok_and(|now| now.as_secs() < expires)
        })
    };
    if !unexpired() || generation_until(service, deadline)? != generation || !unexpired() {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "HTTP delivery authorization changed",
        ));
    }
    Ok(())
}
