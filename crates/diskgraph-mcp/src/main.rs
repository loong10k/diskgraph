//! The DiskGraph MCP server binary. On stdio the protocol frames go to stdout
//! and logging to stderr (spec MCP-02); on Streamable HTTP the same service is
//! served over loopback (spec MCP-01).

use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use diskgraph_mcp::McpService;
use diskgraph_mcp::protocol::ToolProfile;
use diskgraph_mcp::{McpConfig, ScanWorkerSettings, http, serve_stdio};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod managed_service_host;
mod scan_worker_shutdown;
#[cfg(unix)]
mod termination_signal;

#[cfg(windows)]
mod auth_key_acl;

fn main() -> ExitCode {
    let mut data_dir = PathBuf::from("diskgraph-data");
    let mut profile = ToolProfile::ReadFull;
    let mut transport = String::from("stdio");
    let mut host = String::from("127.0.0.1");
    let mut port: u16 = 0;
    // Bearer-token verification for the streamable-http transport. None means
    // the server only binds loopback and authenticates nobody; a non-loopback
    // bind requires one.
    let mut issuer: Option<(String, String, String)> = None;
    // Asserts that TLS (or an equivalent encrypted tunnel) terminates in front
    // of this server; without it, non-loopback binds are refused because this
    // HTTP core is plaintext.
    let mut secure_transport = false;
    let mut allowed_origins: Vec<String> = Vec::new();
    let mut allow_null_origin = false;
    let mut trusted_proxies: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => match args.next() {
                Some(value) => data_dir = PathBuf::from(value),
                None => return usage("--data-dir requires a path"),
            },
            "--profile" => match args.next().as_deref().map(ToolProfile::parse) {
                Some(Some(value)) => profile = value,
                _ => return usage("--profile must be read-minimal|read-full|manage|all"),
            },
            "--transport" => match args.next() {
                Some(value) => transport = value,
                None => return usage("--transport requires a value"),
            },
            "--host" => match args.next() {
                Some(value) => host = value,
                None => return usage("--host requires an address"),
            },
            "--port" => match args.next().as_deref().map(str::parse::<u16>) {
                Some(Ok(value)) => port = value,
                _ => return usage("--port requires a number"),
            },
            // --auth ISSUER AUDIENCE KEY: verify HS256 bearer tokens from that
            // issuer for that audience with that verification key.
            "--auth" => {
                let (Some(iss), Some(aud), Some(key)) = (args.next(), args.next(), args.next())
                else {
                    return usage("--auth requires ISSUER AUDIENCE KEY");
                };
                if issuer.is_some() {
                    return usage("configure only one authentication source");
                }
                issuer = Some((iss, aud, key));
            }
            "--auth-key-file" => {
                let (Some(iss), Some(aud), Some(path)) = (args.next(), args.next(), args.next())
                else {
                    return usage("--auth-key-file requires ISSUER AUDIENCE PATH");
                };
                if issuer.is_some() {
                    return usage("configure only one authentication source");
                }
                let key = match read_auth_key(Path::new(&path)) {
                    Ok(key) => key,
                    Err(error) => {
                        eprintln!("invalid authentication key file: {error}");
                        return ExitCode::from(6);
                    }
                };
                issuer = Some((iss, aud, key));
            }
            "--secure-transport" => secure_transport = true,
            "--allowed-origin" => match args.next() {
                Some(value) => allowed_origins.push(value),
                None => return usage("--allowed-origin requires a value"),
            },
            "--allow-null-origin" => allow_null_origin = true,
            "--trusted-proxy" => match args.next() {
                Some(value) => trusted_proxies.push(value),
                None => return usage("--trusted-proxy requires an address"),
            },
            "--version" | "-V" => {
                println!("diskgraph-mcp {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => return usage(""),
            other => return usage(&format!("unknown argument: {other}")),
        }
    }
    if transport != "stdio" && transport != "streamable-http" && transport != "legacy-sse" {
        eprintln!("unsupported transport {transport}; use stdio, streamable-http, or legacy-sse");
        return ExitCode::from(2);
    }
    let legacy_sse = transport == "legacy-sse";
    let authenticator = issuer.as_ref().map(|(iss, aud, key)| {
        diskgraph_mcp::auth::Authenticator::new(diskgraph_mcp::auth::AuthConfig::single(
            iss,
            aud,
            key.as_bytes(),
        ))
    });
    if transport != "stdio" {
        if authenticator.is_none() {
            eprintln!(
                "HTTP and SSE require --auth ISSUER AUDIENCE KEY, including loopback listeners"
            );
            return ExitCode::from(6);
        }
        use diskgraph_mcp::http::BindPolicy;
        match http::bind_decision(&host, authenticator.is_some(), secure_transport) {
            BindPolicy::LoopbackPlaintext | BindPolicy::NetworkWithTunnel => {}
            BindPolicy::RefuseNoAuth => {
                eprintln!("refusing to bind {host} without --auth ISSUER AUDIENCE KEY");
                return ExitCode::from(6);
            }
            BindPolicy::RefusePlaintext => {
                eprintln!(
                    "refusing to bind {host}: this server is plaintext HTTP; \
                     serve it behind TLS or an encrypted tunnel and pass --secure-transport"
                );
                return ExitCode::from(6);
            }
        }
    }
    let network_policy = http::NetworkPolicy {
        allowed_origins,
        allow_null_origin,
        trusted_proxies,
    };

    #[cfg(unix)]
    let termination = if transport != "stdio" {
        match termination_signal::TerminationSignal::install() {
            Ok(guard) => Some(guard),
            Err(error) => {
                eprintln!("cannot install HTTP shutdown signals: {error}");
                return ExitCode::from(10);
            }
        }
    } else {
        None
    };

    // 部署材料来自本地环境，须在创建数据库前完整拒绝部分/非法配置。
    let worker_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let worker_host = match diskgraph_engine::ScanWorkerRuntimeBudget::new(
        diskgraph_scan_worker::ProtocolLimits {
            max_frame_bytes: 1 << 20,
            max_stream_bytes: 2 << 30,
            max_nodes: 2_000_000,
            max_depth: 4096,
        },
        64 << 10,
        1,
    )
    .and_then(|runtime| {
        ScanWorkerSettings::host_from_environment(runtime, worker_deadline, &mut || Ok(()))
    }) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("invalid scan worker deployment: {error}");
            return ExitCode::from(10);
        }
    };
    #[cfg(unix)]
    if termination
        .as_ref()
        .is_some_and(termination_signal::TerminationSignal::requested)
    {
        // 尚未创建服务或恢复责任；停止请求不再进入数据库/bootstrap。
        return ExitCode::SUCCESS;
    }
    let config = McpConfig {
        data_dir,
        profile,
        legacy_sse,
        ..McpConfig::default()
    };
    #[cfg(windows)]
    let (probe_host, probe_recovery) = match diskgraph_engine::ProbeHost::new(1) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("invalid probe deployment: {error}");
            return ExitCode::from(10);
        }
    };
    #[cfg(windows)]
    let opened = McpService::open_with_process_hosts_until(
        config,
        worker_host,
        probe_host,
        transport == "stdio",
        worker_deadline,
    );
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let opened = McpService::open_with_host_until(
        config,
        worker_host,
        transport == "stdio",
        worker_deadline,
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let opened = managed_service_host::ManagedServiceHost::open(
        config,
        worker_host,
        transport == "stdio",
        worker_deadline,
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (mut service, recovery, slot) = match opened {
        Ok(host) => (host.service, host.recovery, host.slot),
        Err(error) => {
            eprintln!("failed to open the DiskGraph service: {error}");
            return match error.primary() {
                diskgraph_engine::EngineError::Business(business) => {
                    ExitCode::from(business.exit_code())
                }
                _ => ExitCode::from(10),
            };
        }
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let (mut service, recovery) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("failed to open the DiskGraph service: {error}");
            return ExitCode::from(10);
        }
    };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if termination
        .as_ref()
        .is_some_and(termination_signal::TerminationSignal::requested)
    {
        // 构造期间到达的终止请求仍消费同一原材料；不启动 runner，不提前丢掉 ACTIVE。
        if let Some(slot) = slot {
            scan_worker_shutdown::finish_original(service.into_supervisor_parts(recovery, slot));
        } else {
            scan_worker_shutdown::finish(recovery.as_ref());
        }
        return ExitCode::SUCCESS;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let retirement_service = slot.as_ref().map(|_| service.clone());

    // Jobs requested over any transport progress without their connection;
    // the runner outlives every socket (MCP-05).
    #[cfg(windows)]
    let probe_recovery = std::sync::Arc::new(probe_recovery);
    #[cfg(windows)]
    let job_runner = match service
        .start_job_runner_with_probe_recovery(std::sync::Arc::clone(&probe_recovery))
    {
        Ok(runner) => runner,
        Err(error) => {
            // 尚未启动runner或协议分发，没有child出生；拒绝错配的恢复责任。
            eprintln!("invalid probe recovery binding: {error}");
            return ExitCode::from(10);
        }
    };
    #[cfg(not(windows))]
    let job_runner = service.start_job_runner();
    // 唯一恢复句柄在协议执行及其 unwind 边界之外存活。
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if transport != "stdio" {
            let (listener, address) = match http::bind(&host, port) {
                Ok(bound) => bound,
                Err(error) => {
                    eprintln!("failed to bind {host}:{port}: {error}");
                    return ExitCode::from(10);
                }
            };
            eprintln!("diskgraph-mcp listening on {address}{}", http::MCP_ENDPOINT);
            let limits = http::HttpLimits::default();
            let security = http::Security::remote(
                authenticator
                    .as_ref()
                    .map(|auth| std::sync::Arc::new(auth.clone())),
            );
            let security = http::Security {
                policy: network_policy,
                ..security
            };
            let config = if legacy_sse {
                http::ServerConfig::modern(limits, security).with_legacy()
            } else {
                http::ServerConfig::modern(limits, security)
            };
            #[cfg(unix)]
            return match diskgraph_mcp::HttpServerRuntime::start(
                service,
                listener,
                config,
                io::stderr(),
            ) {
                Ok(runtime) => {
                    while !runtime.is_finished()
                        && !termination.as_ref().expect("HTTP signal guard").requested()
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    match runtime.stop_and_join() {
                        Ok(Ok(_)) => ExitCode::SUCCESS,
                        Ok(Err(error)) => {
                            eprintln!("http loop failed: {error}");
                            ExitCode::from(10)
                        }
                        Err(payload) => std::panic::resume_unwind(payload),
                    }
                }
                Err(error) => {
                    eprintln!("http startup failed: {error}");
                    ExitCode::from(10)
                }
            };
            #[cfg(not(unix))]
            return match http::serve_config(service, listener, config, io::stderr()) {
                Ok(_) => ExitCode::from(0),
                Err(error) => {
                    eprintln!("http loop failed: {error}");
                    ExitCode::from(10)
                }
            };
        }

        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let stderr = io::stderr();
        match serve_stdio(
            &mut service,
            BufReader::new(stdin.lock()),
            &mut stdout,
            stderr.lock(),
        ) {
            Ok(()) => ExitCode::from(0),
            Err(error) => {
                eprintln!("stdio loop failed: {error}");
                ExitCode::from(10)
            }
        }
    }));
    // 先停止调度并 join runner，才能处置仍保留的原 OS owner。
    let runner_outcome = job_runner.stop_and_join();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    match (retirement_service, slot) {
        (Some(original), Some(slot)) => {
            scan_worker_shutdown::finish_original(original.into_supervisor_parts(recovery, slot))
        }
        (None, None) => scan_worker_shutdown::finish(recovery.as_ref()),
        _ => unreachable!("original service and slot are created together"),
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    scan_worker_shutdown::finish(
        recovery.as_ref(),
        #[cfg(windows)]
        &probe_recovery,
    );
    match outcome {
        Ok(code) => match runner_outcome {
            Ok(()) => code,
            Err(payload) => std::panic::resume_unwind(payload),
        },
        // 协议异常先发生，仍保留它为主异常；后台 join 已完成且资源恢复已执行。
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn usage(problem: &str) -> ExitCode {
    if !problem.is_empty() {
        eprintln!("{problem}");
    }
    eprintln!(
        "diskgraph-mcp — MCP server\n\
         \n\
         USAGE:\n    diskgraph-mcp [--version] [--data-dir PATH] [--profile PROFILE]\n\
         \x20                     [--transport stdio|streamable-http] [--host ADDR] [--port PORT]\n\
         \x20                     [--auth-key-file ISSUER AUDIENCE PATH]\n\
         \n\
         TRANSPORT:\n    stdio             local process; protocol on stdout, logs on stderr\n    \
         streamable-http   HTTP server; loopback hosts only\n    \
         legacy-sse        same HTTP server with the legacy SSE adapter on\n\
         \x20                 (off by default; opted in by this transport name)\n\
         \n\
         PROFILE:\n    read-minimal  common read tools only\n    \
         read-full     every metadata query\n    \
         manage        adds scope and index management\n    \
         all           everything served in this build"
    );
    if problem.is_empty() {
        ExitCode::from(0)
    } else {
        ExitCode::from(2)
    }
}

fn read_auth_key(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "authentication key must be a nonempty regular file of at most 4096 bytes",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "authentication key file must not be accessible by group or others",
            ));
        }
    }
    #[cfg(windows)]
    auth_key_acl::ensure_restricted(&file)?;
    let mut key = String::new();
    file.take(4097).read_to_string(&mut key)?;
    let key = key.trim_end_matches(['\r', '\n']);
    if key.len() < 32 || key.len() > 4096 || key.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "authentication key must contain 32 to 4096 UTF-8 bytes",
        ));
    }
    Ok(key.to_owned())
}
