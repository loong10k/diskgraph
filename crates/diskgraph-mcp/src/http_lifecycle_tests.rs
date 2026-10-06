//! 真实 socket 停止与唯一线程 owner 回归；来源：原生 Rust PF-06。
use crate::http::{HttpLimits, Security, ServerConfig};
use crate::http_server_runtime::HttpServerRuntime;
use crate::protocol::ToolProfile;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

fn start(
    legacy: bool,
) -> (
    crate::tests::McpTestDirectory,
    HttpServerRuntime,
    std::net::SocketAddr,
) {
    let (service, directory) = crate::tests::service(ToolProfile::ReadFull, "lifecycle");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let config = ServerConfig::modern(
        HttpLimits {
            read_timeout: Duration::from_secs(60),
            ..HttpLimits::default()
        },
        Security::local(None),
    );
    let config = if legacy { config.with_legacy() } else { config };
    let runtime = HttpServerRuntime::start(service, listener, config, Vec::new()).unwrap();
    (directory, runtime, address)
}

fn connect(address: std::net::SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

fn assert_closed(mut stream: impl Read) {
    let mut bytes = Vec::new();
    match stream.read_to_end(&mut bytes) {
        Ok(_) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        Err(error) => panic!("held socket did not close: {error}"),
    }
}

#[test]
fn stop_joins_idle_accept_and_releases_original_listener() {
    let (_directory, runtime, address) = start(false);
    assert_eq!(runtime.stop_and_join().unwrap().unwrap(), 0);
    assert!(TcpStream::connect(address).is_err());
}

#[test]
fn stop_closes_accepted_incomplete_request_before_long_read_timeout() {
    let (_directory, runtime, address) = start(false);
    let mut stream = connect(address);
    stream.write_all(b"POST /mcp HTTP/1.1\r\nHost:").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.active_connections() != 1 {
        assert!(Instant::now() < deadline, "request never admitted");
        std::thread::sleep(Duration::from_millis(1));
    }
    let started = Instant::now();
    runtime.stop_and_join().unwrap().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "shutdown relied on 60-second read timeout"
    );
    assert_closed(stream);
    assert!(TcpStream::connect(address).is_err());
}

fn stop_stream(legacy: bool) {
    let (_directory, runtime, address) = start(legacy);
    let mut stream = connect(address);
    let path = if legacy { "/sse" } else { "/mcp" };
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.starts_with("HTTP/1.1 200"), "{line}");
    loop {
        line.clear();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        if line == "\r\n" {
            break;
        }
    }
    runtime.stop_and_join().unwrap().unwrap();
    assert_closed(reader);
    assert!(TcpStream::connect(address).is_err());
}

#[test]
fn stop_joins_modern_sse_with_client_still_connected() {
    stop_stream(false);
}

#[test]
fn stop_joins_legacy_sse_with_client_still_connected() {
    stop_stream(true);
}

#[test]
fn foreground_panic_raii_joins_original_server_before_directory_cleanup() {
    let (_directory, runtime, address) = start(false);
    let stream = connect(address);
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _runtime = runtime;
        std::panic::panic_any(String::from("original HTTP fixture panic"));
    }))
    .unwrap_err();
    assert_eq!(
        failure.downcast_ref::<String>().map(String::as_str),
        Some("original HTTP fixture panic")
    );
    assert_closed(stream);
    assert!(TcpStream::connect(address).is_err());
}
