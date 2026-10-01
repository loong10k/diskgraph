//! The DiskGraph MCP server binary. On stdio the protocol frames go to stdout
//! and logging to stderr (spec MCP-02); on Streamable HTTP the same service is
//! served over loopback (spec MCP-01).

use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use diskgraph_mcp::protocol::ToolProfile;
use diskgraph_mcp::{McpConfig, McpService, http, serve_stdio};

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

    let open_service = if transport == "stdio" {
        McpService::open
    } else {
        McpService::open_remote
    };
    let mut service = match open_service(McpConfig {
        data_dir,
        profile,
        legacy_sse,
        ..McpConfig::default()
    }) {
        Ok(service) => service,
        Err(error) => {
            eprintln!("failed to open the DiskGraph service: {error}");
            return ExitCode::from(10);
        }
    };

    // Jobs requested over any transport progress without their connection;
    // the runner outlives every socket (MCP-05).
    let _job_runner = service.start_job_runner();

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
