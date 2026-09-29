//! Isolated filesystem fixtures for P0 tasks 1.2 and 1.7 (specs FS-02 /
//! FS-03 / FS-04 / FS-05 / RE-02).
//!
//! Every fixture builds inside its own temp directory and never touches user
//! data. Scenarios that need a real OS feature we cannot fabricate (cloud
//! placeholders, mount replacement) are listed by [`real_os_requirements`]
//! instead of being faked.

use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// A disposable directory tree for scanner and store tests.
pub struct FixtureTree {
    root: TempDir,
}

impl FixtureTree {
    /// Creates the empty fixture root.
    pub fn new(label: &str) -> io::Result<Self> {
        Ok(Self {
            root: TempDir::with_prefix(format!("diskgraph-{label}-"))?,
        })
    }

    /// The fixture root path.
    pub fn path(&self) -> &Path {
        self.root.path()
    }

    /// Creates a subdirectory inside the fixture.
    pub fn dir(&self, relative: &str) -> io::Result<PathBuf> {
        let path = self.root.path().join(relative);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Writes a regular file of `size` bytes (zeros).
    pub fn file(&self, relative: &str, size: usize) -> io::Result<PathBuf> {
        let path = self.root.path().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, vec![0_u8; size])?;
        Ok(path)
    }

    /// Writes a dot-hidden file.
    pub fn hidden_file(&self, relative: &str, size: usize) -> io::Result<PathBuf> {
        let name = format!(".{relative}");
        self.file(&name, size)
    }

    /// Creates a hard link to an existing fixture file.
    pub fn hardlink(&self, original: &str, link: &str) -> io::Result<PathBuf> {
        let from = self.root.path().join(original);
        let to = self.root.path().join(link);
        std::fs::hard_link(&from, &to)?;
        Ok(to)
    }

    /// Creates a symlink (including deliberately looping ones); not followed
    /// by the scanner under the default settings.
    pub fn symlink(&self, target: &Path, link: &str) -> io::Result<PathBuf> {
        let to = self.root.path().join(link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &to)?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, &to)?;
        Ok(to)
    }

    /// Creates a sparse file: `apparent_len` apparent bytes, near-zero blocks.
    /// Unix only; on other platforms it degrades to a regular file so callers
    /// can skip the allocation assertions.
    #[cfg(unix)]
    pub fn sparse_file(&self, relative: &str, apparent_len: u64) -> io::Result<PathBuf> {
        let path = self.root.path().join(relative);
        let file = std::fs::File::create(&path)?;
        file.set_len(apparent_len)?;
        Ok(path)
    }

    /// Creates a directory whose bytes-name is not valid UTF-8 (FS-02 fixture).
    #[cfg(unix)]
    pub fn non_utf8_name(&self, suffix: &[u8]) -> io::Result<PathBuf> {
        use std::os::unix::ffi::OsStringExt;
        let mut raw = b"raw-name-".to_vec();
        raw.extend_from_slice(suffix);
        let path = self.root.path().join(std::ffi::OsString::from_vec(raw));
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }
}

/// Guard restoring directory permissions on drop (POSIX only).
#[cfg(unix)]
pub struct UnreadableDir {
    path: PathBuf,
}

#[cfg(unix)]
impl UnreadableDir {
    /// Makes `path` unreadable (mode 000) for the lifetime of the guard.
    pub fn make(path: PathBuf) -> io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_mode(0o000);
        std::fs::set_permissions(&path, permissions)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Drop for UnreadableDir {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut permissions) = std::fs::metadata(&self.path).map(|m| m.permissions()) {
            permissions.set_mode(0o755);
            let _ = std::fs::set_permissions(&self.path, permissions);
        }
    }
}

/// Scenarios that require a real OS feature and must be validated on dedicated
/// hosts, never faked in unit tests (RE-02).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealOsRequirement {
    /// A cloud placeholder (macOS file provider) that must not hydrate on scan.
    CloudPlaceholder,
    /// A volume swapped/replaced under an already-open path.
    MountReplacement,
    /// A real recycling backend per platform (Finder Trash, FreeDesktop, Recycle Bin).
    PlatformTrashBackend,
    /// Android/iOS provider grants and revocations on real devices.
    MobileProviderLifecycle,
}

impl RealOsRequirement {
    pub fn describe(self) -> &'static str {
        match self {
            Self::CloudPlaceholder => {
                "requires a real cloud file provider; scans must record a coverage limit instead of downloading"
            }
            Self::MountReplacement => {
                "requires an actual mount swap under an open path; unit fixtures cannot race a mount"
            }
            Self::PlatformTrashBackend => {
                "requires the platform trash service; failure must report unsupported, never delete"
            }
            Self::MobileProviderLifecycle => {
                "requires real Android/iOS devices to grant and revoke provider access"
            }
        }
    }
}

/// The machine-checkable manifest of scenarios deferred to real-OS validation.
pub fn real_os_requirements() -> &'static [(RealOsRequirement, &'static str)] {
    &[
        (RealOsRequirement::CloudPlaceholder, "FS-05 / CT-02"),
        (RealOsRequirement::MountReplacement, "FS-04 / OP-04"),
        (RealOsRequirement::PlatformTrashBackend, "OP-06"),
        (RealOsRequirement::MobileProviderLifecycle, "PF-04 / PF-05"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_tree_builds_files_dirs_and_hidden_entries() {
        let tree = FixtureTree::new("basic").unwrap();
        let file = tree.file("sub/data.bin", 128).unwrap();
        assert!(file.is_file());
        assert!(tree.dir("empty-dir").unwrap().is_dir());
        let hidden = tree.hidden_file("secret", 4).unwrap();
        assert!(hidden.is_file());
        assert_eq!(std::fs::metadata(&file).unwrap().len(), 128);
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_points_at_the_same_inode() {
        use std::os::unix::fs::MetadataExt;
        let tree = FixtureTree::new("hardlink").unwrap();
        let original = tree.file("a.bin", 64).unwrap();
        let link = tree.hardlink("a.bin", "b.bin").unwrap();
        assert_eq!(
            std::fs::metadata(&original).unwrap().ino(),
            std::fs::metadata(&link).unwrap().ino()
        );
    }

    #[cfg(unix)]
    #[test]
    fn sparse_file_has_apparent_bytes_far_above_allocation() {
        use std::os::unix::fs::MetadataExt;
        let tree = FixtureTree::new("sparse").unwrap();
        let path = tree.sparse_file("hole.bin", 1 << 20).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.len(), 1 << 20);
        assert!(
            metadata.blocks() * 512 < (1 << 20),
            "expected sparse allocation, got {} blocks",
            metadata.blocks()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_dir_guard_restores_permissions_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let tree = FixtureTree::new("perm").unwrap();
        let path = tree.dir("locked").unwrap();
        {
            let guard = UnreadableDir::make(path.clone()).unwrap();
            let mode = std::fs::metadata(guard.path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0);
        }
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn real_os_requirements_manifest_is_machine_checkable() {
        let requirements = real_os_requirements();
        assert!(requirements.len() >= 4);
        for (requirement, specs) in requirements {
            assert!(!requirement.describe().is_empty());
            assert!(
                specs.starts_with("FS")
                    || specs.starts_with("CT")
                    || specs.starts_with("OP")
                    || specs.starts_with("PF")
            );
        }
    }
}

/// A minimal HTTP/1.1 client for transport tests: it speaks the wire directly
/// so a passing test proves the server, not a client library's leniency.
pub mod http_client {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    /// Sends one POST with a JSON body and returns the response body.
    pub fn post_json(port: u16, path: &str, body: &str) -> String {
        send(port, "POST", path, body, &[])
    }

    /// Sends one GET and returns the response body.
    pub fn get(port: u16, path: &str) -> String {
        send(port, "GET", path, "", &[])
    }

    /// Sends one request with extra headers and returns the response body.
    pub fn send(
        port: u16,
        method: &str,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the server");
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("Connection: close\r\n\r\n");
        stream
            .write_all(head.as_bytes())
            .expect("write request head");
        stream
            .write_all(body.as_bytes())
            .expect("write request body");
        stream.flush().expect("flush request");

        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).expect("read status line");
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        if length > 0 {
            reader.read_exact(&mut body).expect("read response body");
        }
        String::from_utf8_lossy(&body).into_owned()
    }
}

/// A client for the legacy (pre-Streamable) MCP HTTP+SSE protocol, written
/// against the 2024-11-05 wire contract: GET /sse, follow the endpoint event,
/// POST messages, read `message` events. Used to prove the legacy adapter
/// speaks what an old host would speak, without needing that host installed.
pub mod legacy_client {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    /// One open SSE stream with its session's message endpoint.
    pub struct SseStream {
        reader: BufReader<TcpStream>,
        pub message_endpoint: String,
    }

    impl SseStream {
        /// Connects and waits for the endpoint event, as a legacy client must.
        pub fn connect(port: u16) -> std::io::Result<Self> {
            let mut stream = TcpStream::connect(("127.0.0.1", port))?;
            stream.write_all(
                b"GET /sse HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
            )?;
            stream.flush()?;
            let mut reader = BufReader::new(stream);
            let mut status = String::new();
            reader.read_line(&mut status)?;
            if !status.contains("200") {
                return Err(std::io::Error::other(format!(
                    "SSE handshake failed: {status}"
                )));
            }
            // Consume headers through the blank line.
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
                    break;
                }
            }
            // The first event must be the endpoint event.
            let (event, data) = read_event(&mut reader)?;
            if event != "endpoint" {
                return Err(std::io::Error::other(format!(
                    "expected endpoint event, got {event:?}"
                )));
            }
            Ok(Self {
                reader,
                message_endpoint: data,
            })
        }

        /// Reads the next event; keep-alives are skipped transparently.
        pub fn next_message(&mut self) -> std::io::Result<String> {
            loop {
                let (event, data) = read_event(&mut self.reader)?;
                if event == "message" {
                    return Ok(data);
                }
            }
        }
    }

    /// Reads one SSE event (event name + joined data lines).
    fn read_event(reader: &mut BufReader<TcpStream>) -> std::io::Result<(String, String)> {
        let mut event = "message".to_owned();
        let mut data = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Err(std::io::Error::other("SSE stream ended"));
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if data.is_empty() && event == "message" {
                    // A bare blank line before any content: keep reading.
                    continue;
                }
                return Ok((event, data));
            }
            if let Some(rest) = line.strip_prefix(':') {
                let _ = rest; // comments and keep-alives carry no content
            } else if let Some(rest) = line.strip_prefix("event:") {
                event = rest.trim().to_owned();
            } else if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.trim_start());
            }
        }
    }

    /// POSTs one JSON-RPC body to the session endpoint on a fresh connection,
    /// returning the HTTP status line.
    pub fn post_message(
        port: u16,
        endpoint: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> std::io::Result<u16> {
        let mut stream = TcpStream::connect(("127.0.0.1", port))?;
        let mut head = format!(
            "POST {endpoint} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("Connection: close\r\n\r\n");
        stream.write_all(head.as_bytes())?;
        stream.write_all(body.as_bytes())?;
        stream.flush()?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status)?;
        // Drain whatever remains so the close is clean.
        let mut rest = String::new();
        let _ = reader.read_to_string(&mut rest);
        let code = status
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        Ok(code)
    }
}
