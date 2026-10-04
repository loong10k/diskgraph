//! CLI service_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use diskgraph_core::BusinessError;
use diskgraph_engine::{Engine, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(engine: &Engine, cli: &Cli, out: &mut Vec<String>) -> Result<(), EngineError> {
    match &cli.command {
        Command::Serve {
            transport,
            profile,
            host,
            port,
            auth,
            auth_key_file,
            allowed_origin,
            allow_null_origin,
            secure_transport,
            trusted_proxy,
        } => {
            let Some(profile) = diskgraph_mcp::protocol::ToolProfile::parse(profile) else {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            };
            // `serve` execs the MCP server so this process's stdout stays clean
            // and the child owns the transport.
            let binary = std::env::current_exe()
                .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))?
                .with_file_name(if cfg!(windows) {
                    "diskgraph-mcp.exe"
                } else {
                    "diskgraph-mcp"
                });
            let mut child = std::process::Command::new(binary);
            child
                .arg("--data-dir")
                .arg(&cli.data_dir)
                .arg("--profile")
                .arg(profile.wire_name())
                .arg("--transport")
                .arg(transport)
                .arg("--host")
                .arg(host)
                .arg("--port")
                .arg(port.to_string());
            if let Some(auth) = auth {
                child.arg("--auth").args(auth);
            }
            if let Some(auth_key_file) = auth_key_file {
                child.arg("--auth-key-file").args(auth_key_file);
            }
            for origin in allowed_origin {
                child.arg("--allowed-origin").arg(origin);
            }
            if *allow_null_origin {
                child.arg("--allow-null-origin");
            }
            if *secure_transport {
                child.arg("--secure-transport");
            }
            for proxy in trusted_proxy {
                child.arg("--trusted-proxy").arg(proxy);
            }
            let status = child
                .status()
                .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))?;
            // The child owns the streams; propagate its exit code.
            std::process::exit(status.code().unwrap_or(10) as i32);
        }
        Command::Doctor => {
            let profile = diskgraph_mcp::protocol::ToolProfile::ReadFull;
            let report = diskgraph_mcp::doctor::diagnose(engine, profile);
            // A degraded report is data, not a tool failure.
            out.push(envelope_line(engine, Ok(report.to_json(engine, profile))));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
