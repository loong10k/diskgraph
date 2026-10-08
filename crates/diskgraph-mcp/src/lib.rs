//! MCP 服务聚合入口；来源：原生 Rust RT-10。各传输共享同一 Engine 与请求授权链。
pub mod auth;
mod bounded_json_writer;
#[cfg(test)]
mod children_cursor_tests;
mod client_address;
mod connection_rejection;
pub mod doctor;
mod error_reply;
mod evidence_job_status;
#[cfg(test)]
mod history_budget_tests;
pub mod http;
mod http_connections;
#[cfg(test)]
mod http_lifecycle_tests;
mod http_limits;
mod http_request;
#[cfg(test)]
mod http_request_syntax_tests;
mod http_response;
mod http_server_config;
mod http_server_runtime;
mod http_shutdown_state;
pub mod install;
mod job_entry;
pub mod legacy;
mod legacy_delivery_error;
mod legacy_delivery_registry;
mod legacy_delivery_session;
mod legacy_delivery_state;
mod legacy_frame;
mod legacy_reservation;
mod legacy_session_receiver;
mod legacy_transport;
pub mod protocol;
mod rate_limit_state;
mod rate_limiter;
#[cfg(test)]
mod relation_budget_tests;
#[cfg(test)]
mod relation_encoding_tests;
mod relation_reply;
mod request_authorizer;
mod request_context;
mod snapshot_reply;
mod sse_slot;
#[cfg(test)]
mod status_deadline_tests;
mod token_bucket;
mod tool_input_schema;

mod error_mapping;
mod filesystem_tools;
mod management_tools;
mod mcp_config;
mod mcp_service;
mod relation_tools;
mod scan_worker_settings;
mod scope_access;
mod service_dispatch;
mod service_identity;
mod snapshot_tools;
mod stdio_service;
#[cfg(test)]
mod tests;

pub(crate) use error_mapping::business_of;
pub use http_server_runtime::HttpServerRuntime;
pub use mcp_config::{McpConfig, STDIO_PRINCIPAL};
pub use mcp_service::McpService;
pub use scan_worker_settings::ScanWorkerSettings;
pub use stdio_service::serve_stdio;

#[cfg(test)]
mod engine_error_cleanup_tests;

#[cfg(test)]
mod sse_identity_deadline_tests;

mod http_debug_request;

#[cfg(test)]
mod http_debug_request_tests;

#[cfg(test)]
mod http_request_audit_tests;

mod http_delivery_writer;
#[cfg(test)]
mod http_delivery_writer_tests;

mod http_delivery_authority;

#[cfg(test)]
mod filesystem_deadline_tests;

#[cfg(test)]
mod request_deadline_tests;

#[cfg(test)]
mod management_deadline_tests;

#[cfg(test)]
mod principal_policy_tests;
