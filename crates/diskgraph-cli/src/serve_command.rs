use crate::cli::Cli;
use crate::command::Command;
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use std::process::ExitStatus;

/// 直接转交 MCP 传输，不打开 CLI 数据库或引导本地管理员。
/// 来源：原生 Rust CLI serve/SC-01；实际 MCP 自行执行本地或远程启动边界。
/// 参数：cli 为已解析的服务选项；返回：真实伴随进程状态或保留原 I/O 错误。
pub(crate) fn run(cli: &Cli) -> Result<ExitStatus, EngineError> {
    let Command::Serve {
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
    } = &cli.command
    else {
        return Err(BusinessError::InvalidArgument.into());
    };
    let Some(profile) = diskgraph_mcp::protocol::ToolProfile::parse(profile) else {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    };
    // 启动并等待真实 MCP 伴随进程，传输直接继承标准流；此入口不建立 CLI Engine。
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
    child
        .status()
        .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))
}
