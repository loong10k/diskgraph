use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::McpService;
use crate::auth::{AuthFailure, AuthenticatedPrincipal, token_from_headers, unauthorized_body};
use crate::bounded_json_writer::BoundedJsonWriter;
use crate::http::{
    HttpLimits, HttpRequest, HttpResponse, OriginDecision, Security, write_response,
};
use crate::legacy_delivery_error::LegacyDeliveryError;
use crate::legacy_delivery_registry::{FRAME_OVERHEAD, LegacyDeliveryRegistry};
use crate::protocol::{decode_request, log_line, protocol_error};

/// legacy HTTP/SSE 适配器，将授权、业务前准入和有界结果投递连成一条路径。
/// 来源：DiskGraph 原生 Rust legacy 协议；无 Java 对应对象。
#[derive(Clone)]
pub(crate) struct LegacyTransport {
    registry: LegacyDeliveryRegistry,
    limits: HttpLimits,
}

impl LegacyTransport {
    /// 创建适配器。参数：limits 为监听器响应/时间预算；返回：共享投递状态的适配器。
    pub(crate) fn new(limits: HttpLimits) -> Self {
        Self {
            registry: LegacyDeliveryRegistry::new(),
            limits,
        }
    }

    /// 持有 SSE 流。参数：stream/identity 是已认证握手，service/log 提供实时策略及记录。
    /// 返回：无，结束时关闭会话并释放剩余队列；每帧发送受绝对期限和授权变更计数检查约束。
    pub(crate) fn serve_stream(
        &self,
        stream: &mut TcpStream,
        identity: Option<&AuthenticatedPrincipal>,
        service: &McpService,
        log: &Arc<Mutex<impl Write + Send>>,
    ) {
        let Ok((session_id, receiver)) = self
            .registry
            .open(identity.map(|identity| identity.principal.as_str().to_owned()))
        else {
            let _ = write_response(
                stream,
                &HttpResponse::json(503, json!({"error":"legacy_unavailable"})),
            );
            return;
        };
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
        if stream.write_all(head.as_bytes()).is_err()
            || stream
                .write_all(crate::legacy::endpoint_event(&session_id).as_bytes())
                .is_err()
            || stream.flush().is_err()
            || stream.set_nonblocking(true).is_err()
        {
            return;
        }
        loop {
            if identity
                .is_some_and(|identity| !crate::http::stream_identity_valid(service, identity))
            {
                break;
            }
            match receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(frame) => {
                    if self
                        .write_frame(
                            stream,
                            frame.wire.as_bytes(),
                            service,
                            identity,
                            Some(frame.authorization_generation),
                        )
                        .is_err()
                    {
                        break;
                    }
                    // 发送完成后才销毁 frame，额度包含仍在写入的缓冲。
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if self
                        .write_frame(
                            stream,
                            crate::legacy::keepalive_event().as_bytes(),
                            service,
                            identity,
                            None,
                        )
                        .is_err()
                    {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let mut probe = [0u8; 1];
            match stream.peek(&mut probe) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = stream.read(&mut probe);
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        // receiver 的关闭守卫覆盖头写失败、正常关闭、撤权和发送失败。
        drop(receiver);
        let _ = log.lock().map(|mut log| {
            writeln!(
                log,
                "{}",
                log_line("legacy_session_closed", &[("session", &session_id)])
            )
        });
    }

    fn write_frame(
        &self,
        stream: &mut TcpStream,
        mut bytes: &[u8],
        service: &McpService,
        identity: Option<&AuthenticatedPrincipal>,
        authorization_generation: Option<u64>,
    ) -> std::io::Result<()> {
        let deadline = Instant::now() + self.limits.read_timeout;
        while !bytes.is_empty() {
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    "legacy frame write deadline",
                ));
            }
            let authorized = match authorization_generation {
                Some(generation) => {
                    // 握手/业务已检查 grant；计数不变说明授权数据仍相同。
                    // 分段写只读取一个计数和期限，避免每 64 KiB 重建整个策略/范围列表。
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|time| time.as_secs())
                        .unwrap_or(u64::MAX);
                    identity.is_none_or(|identity| now < identity.expires_at_unix_seconds)
                        && Self::generation_until(service, deadline)? == generation
                }
                None => {
                    // keepalive 没有业务内容；完整 grant 检查在外层每次循环完成。
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|time| time.as_secs())
                        .unwrap_or(u64::MAX);
                    identity.is_none_or(|identity| now < identity.expires_at_unix_seconds)
                }
            };
            // 锁等待、SQLite 和调度都消耗同一个期限；授权返回后不能续期发送。
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    "legacy frame write deadline",
                ));
            }
            if !authorized {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "legacy delivery authorization changed",
                ));
            }
            // 非阻塞写允许部分进展；不能把正常 WouldBlock 当作完整帧已丢失。
            match stream.write(&bytes[..bytes.len().min(64 * 1024)]) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        ErrorKind::WriteZero,
                        "legacy stream closed",
                    ));
                }
                Ok(count) => bytes = &bytes[count..],
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        }
        stream.flush()
    }

    fn authorization_generation(service: &McpService) -> Option<u64> {
        service
            .engine()
            .control_store()
            .ok()
            .and_then(|store| store.authorization_generation().ok())
    }

    fn generation_until(service: &McpService, deadline: Instant) -> std::io::Result<u64> {
        while Instant::now() < deadline {
            match service
                .engine()
                .try_control_store()
                .map_err(|_| std::io::Error::other("control store unavailable"))?
            {
                Some(store) => {
                    return store
                        .authorization_generation_until(deadline)
                        .map_err(|error| {
                            let exhausted =
                                matches!(&error, diskgraph_store::StoreError::BudgetExceeded)
                                    || error.is_interrupted();
                            std::io::Error::new(
                                if exhausted {
                                    ErrorKind::TimedOut
                                } else {
                                    ErrorKind::Other
                                },
                                "legacy authorization read failed",
                            )
                        });
                }
                None => std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(1)),
                ),
            }
        }
        Err(std::io::Error::new(
            ErrorKind::TimedOut,
            "legacy authorization read deadline",
        ))
    }

    /// 处理投递请求。参数：stream/request 为 POST，security/service/log 为认证与执行上下文。
    /// 返回：是否继续连接；429/413 在业务前拒绝，202 之后的断线会明确关闭并记录。
    pub(crate) fn post(
        &self,
        stream: &mut TcpStream,
        request: &HttpRequest,
        security: &Security,
        shared_service: &Arc<McpService>,
        log: &Arc<Mutex<impl Write + Send>>,
    ) -> bool {
        if security.policy.origin_decision(request.header("origin")) == OriginDecision::Refused {
            return Self::reply(stream, 403, json!({"error":"forbidden_origin"}));
        }
        let identity = if let Some(authenticator) = &security.authenticator {
            match authenticator.authenticate(token_from_headers(&request.headers).as_deref()) {
                Ok(identity) => Some(identity),
                Err(failure) => {
                    return Self::reply(stream, failure.http_status(), unauthorized_body(failure));
                }
            }
        } else if shared_service.context.trusted_local() {
            None
        } else {
            return Self::reply(stream, 401, unauthorized_body(AuthFailure::Missing));
        };
        let Some(session_id) = request.query_param("session_id") else {
            return Self::reply(stream, 400, json!({"error":"missing_session_id"}));
        };
        let value: Value = match serde_json::from_str(request.body.trim()) {
            Ok(value) => value,
            Err(error) => {
                return Self::reply(
                    stream,
                    400,
                    protocol_error(Value::Null, -32700, &format!("parse error: {error}")),
                );
            }
        };
        if value.is_array() {
            return Self::reply(
                stream,
                400,
                protocol_error(
                    Value::Null,
                    -32600,
                    "batched requests are not supported; send one message per POST",
                ),
            );
        }
        drop(value);
        let Ok(decoded) = decode_request(request.body.trim()) else {
            return Self::reply(
                stream,
                400,
                protocol_error(Value::Null, -32600, "malformed frame"),
            );
        };
        // 最小错误也须保留请求 ID；无法关联的配置/请求在执行前拒绝。
        let Some(overflow) = BoundedJsonWriter::encode(
            &protocol_error(decoded.id.clone(), -32000, "response_too_large"),
            self.limits.max_response_bytes,
        ) else {
            return Self::reply(stream, 413, json!({"error":"response_budget_too_small"}));
        };
        let Some(max_frame) = self.limits.max_response_bytes.checked_add(FRAME_OVERHEAD) else {
            return Self::reply(stream, 413, json!({"error":"legacy_response_limit"}));
        };
        let principal = identity
            .as_ref()
            .map(|identity| identity.principal.as_str());
        let mut reservation = match self.registry.reserve(&session_id, principal, max_frame) {
            Ok(reservation) => reservation,
            Err(error) => {
                let (status, reason) = match error {
                    LegacyDeliveryError::UnknownSession => (404, "unknown_session"),
                    LegacyDeliveryError::PrincipalMismatch => (403, "session_principal_mismatch"),
                    LegacyDeliveryError::ResponseLimit => (413, "legacy_response_limit"),
                    LegacyDeliveryError::Backpressure => (429, "legacy_backpressure"),
                    LegacyDeliveryError::Unavailable => (503, "legacy_unavailable"),
                };
                return Self::reply(stream, status, json!({"error":reason}));
            }
        };
        let Some(authorization_generation) = Self::authorization_generation(shared_service) else {
            return Self::reply(stream, 503, json!({"error":"legacy_unavailable"}));
        };
        let accepted = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n";
        if stream.write_all(accepted.as_bytes()).is_err() || stream.flush().is_err() {
            return false;
        }
        let mut service = shared_service.as_ref().clone();
        let response = if let Some(identity) = &identity {
            service
                .for_transport_identity(identity, "legacy_sse")
                .handle(&decoded)
        } else {
            service.handle(&decoded)
        };
        let mut wire = BoundedJsonWriter::encode(&response, self.limits.max_response_bytes)
            .unwrap_or(overflow);
        drop(response);
        // 消费同一字符串并加包装，不再同时保留 payload 与完整帧两个编码副本。
        wire.reserve_exact(FRAME_OVERHEAD);
        wire.insert_str(0, "event: message\ndata: ");
        wire.push_str("\n\n");
        let result = reservation.shrink(wire.len()).and_then(|()| {
            self.registry.enqueue(crate::legacy_frame::LegacyFrame {
                wire,
                authorization_generation,
                _reservation: reservation,
            })
        });
        if result.is_err() {
            // 准入已保证不会正常满队列。准入后的断线/失效必须显式结束会话。
            self.registry.close(&session_id);
            let _ = log.lock().map(|mut log| {
                writeln!(
                    log,
                    "{}",
                    log_line("legacy_delivery_failed", &[("session", &session_id)])
                )
            });
        }
        true
    }

    fn reply(stream: &mut TcpStream, status: u16, body: Value) -> bool {
        write_response(stream, &HttpResponse::json(status, body)).is_ok()
    }
}

#[cfg(test)]
#[path = "legacy_transport_tests.rs"]
mod tests;
