//! Lossless resource location (P0 task 1.2, spec FS-02).
//!
//! The v1 [`crate::ResourceLocator`] stores paths as Rust `String`, which loses
//! non-UTF-8 bytes through `to_string_lossy()`. The v2 [`Locator`] keeps the raw
//! operating-system encoding (Unix path bytes, Windows UTF-16 code units) as
//! base64 and stores a lossy display string separately. Display strings are
//! never valid operation targets; only raw bytes identify a resource.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::{DecodeError, Engine as _};
use serde::{Deserialize, Serialize};

/// How the raw bytes of a locator must be interpreted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocatorKind {
    /// Operating-system path; raw bytes are platform-encoded.
    NativePath,
    /// An opaque provider URI (Android SAF, iOS document); bytes are the URI text.
    DocumentUri,
}

/// A lossless observation locator. `display` is for humans and logs only.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Locator {
    pub kind: LocatorKind,
    /// Base64 of the platform-encoded resource identity; never a printable claim.
    pub raw_b64: String,
    /// Lossy rendering for display; MUST NOT be used to address files.
    pub display: String,
}

/// The raw bytes could not be decoded back out of a locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocatorDecodeError {
    NotBase64(DecodeError),
    NotUtf8Uri,
}

impl core::fmt::Display for LocatorDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotBase64(error) => write!(f, "locator raw_b64 is not valid base64: {error}"),
            Self::NotUtf8Uri => write!(f, "document URI locator is not valid UTF-8"),
        }
    }
}

impl std::error::Error for LocatorDecodeError {}

impl Locator {
    /// Captures the full platform encoding of a native path.
    pub fn from_native_path(path: &Path) -> Self {
        Self {
            kind: LocatorKind::NativePath,
            raw_b64: BASE64.encode(raw_path_bytes(path)),
            display: path.to_string_lossy().into_owned(),
        }
    }

    /// Wraps an opaque provider URI; the URI text is the identity.
    pub fn from_document_uri(uri: impl AsRef<str>) -> Self {
        Self {
            kind: LocatorKind::DocumentUri,
            raw_b64: BASE64.encode(uri.as_ref().as_bytes()),
            display: uri.as_ref().to_owned(),
        }
    }

    /// Decodes the raw identity bytes; the only legitimate operation target.
    pub fn raw_bytes(&self) -> Result<Vec<u8>, LocatorDecodeError> {
        BASE64
            .decode(self.raw_b64.as_bytes())
            .map_err(LocatorDecodeError::NotBase64)
    }

    /// Decodes the locator back to a native path; fails if the raw bytes are
    /// not valid for the current platform encoding.
    pub fn to_native_path(&self) -> Result<std::path::PathBuf, LocatorDecodeError> {
        if self.kind != LocatorKind::NativePath {
            return Err(LocatorDecodeError::NotUtf8Uri);
        }
        let bytes = self.raw_bytes()?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            Ok(std::ffi::OsString::from_vec(bytes).into())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Ok(std::ffi::OsString::from_wide(&units).into())
        }
        #[cfg(not(any(unix, windows)))]
        {
            String::from_utf8(bytes)
                .map(std::path::PathBuf::from)
                .map_err(|_| LocatorDecodeError::NotUtf8Uri)
        }
    }

    /// The display string; logs and UI only, never an operation target.
    pub fn display(&self) -> &str {
        &self.display
    }
}

fn raw_path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut bytes = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }
    #[cfg(not(any(unix, windows)))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_survive_v2_round_trip_but_not_v1_strings() {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt};
        let lossy_name =
            std::ffi::OsString::from_vec(vec![b'f', b'i', b'l', b'e', 0xFF, b'.', b't']);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(&lossy_name);
        match std::fs::write(&path, b"data") {
            Ok(()) => {
                // Linux (byte-transparent filesystems): the lossless v2 locator
                // must round-trip bytes the v1 string already corrupted.
                let v1 = path.to_string_lossy().into_owned();
                assert!(v1.contains('\u{FFFD}'), "v1 must be shown lossy: {v1}");
                let locator = Locator::from_native_path(&path);
                assert_eq!(locator.display(), v1);
                let restored = locator.to_native_path().unwrap();
                assert_eq!(restored.as_os_str(), lossy_name.as_os_str());
                assert_eq!(locator.raw_bytes().unwrap(), lossy_name.as_bytes());
            }
            Err(error) => {
                // macOS APFS rejects non-UTF-8 names outright (EILSEQ); that is
                // a recorded platform fact, and the encoding layer must still be
                // lossless for such bytes.
                assert_eq!(
                    error.raw_os_error(),
                    Some(92),
                    "unexpected failure creating non-UTF-8 name: {error}"
                );
                let encoded = Locator::from_native_path(&directory.path().join(&lossy_name));
                assert_eq!(
                    encoded.raw_bytes().unwrap(),
                    directory.path().join(&lossy_name).as_os_str().as_bytes()
                );
                assert_eq!(
                    encoded.to_native_path().unwrap().as_os_str(),
                    directory.path().join(&lossy_name).as_os_str()
                );
            }
        }
    }

    #[test]
    fn document_uri_locator_round_trips_and_display_stays_separate() {
        let locator = Locator::from_document_uri("content://docs/self/uuid/children");
        assert_eq!(locator.kind, LocatorKind::DocumentUri);
        assert_eq!(
            locator.raw_bytes().unwrap(),
            b"content://docs/self/uuid/children"
        );
        assert!(locator.to_native_path().is_err());
    }

    #[test]
    fn locator_json_round_trip_preserves_every_field() {
        let locator = Locator::from_native_path(std::path::Path::new("/tmp/ünïcode"));
        let encoded = serde_json::to_string(&locator).unwrap();
        let decoded: Locator = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, locator);
    }

    #[test]
    fn corrupt_base64_is_reported_not_loaned_as_a_display_string() {
        let locator = Locator {
            kind: LocatorKind::NativePath,
            raw_b64: "###not-base64###".into(),
            display: "/tmp/whatever".into(),
        };
        let error = locator.raw_bytes().unwrap_err();
        assert!(matches!(error, LocatorDecodeError::NotBase64(_)));
    }
}
