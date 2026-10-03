//! D30 编码限定定位回归；可信旧 Locator 的现有契约不受影响。
use crate::{LocatorEncoding, LocatorKind, QualifiedLocator};

#[test]
fn qualified_locator_encoding_labels_are_exact_and_round_trip() {
    for (encoding, label) in [
        (LocatorEncoding::UnixBytes, "unix_bytes"),
        (LocatorEncoding::WindowsUtf16Le, "windows_utf16_le"),
        (LocatorEncoding::Utf8Uri, "utf8_uri"),
    ] {
        assert_eq!(encoding.wire_name(), label);
        assert_eq!(LocatorEncoding::parse(label).unwrap(), encoding);
        assert_eq!(
            serde_json::to_string(&encoding).unwrap(),
            format!("\"{label}\"")
        );
        assert_eq!(
            serde_json::from_str::<LocatorEncoding>(&format!("\"{label}\"")).unwrap(),
            encoding
        );
    }
    for label in [
        "",
        "native",
        "utf8",
        "UNIX_BYTES",
        "windows_utf16le",
        "future_encoding",
    ] {
        assert!(LocatorEncoding::parse(label).is_err(), "accepted {label}");
        assert!(serde_json::from_str::<LocatorEncoding>(&format!("\"{label}\"")).is_err());
    }
}

#[test]
fn qualified_locator_native_capture_preserves_host_bytes() {
    let path = std::path::Path::new("fixture-中文");
    let locator = QualifiedLocator::from_native_path(path).unwrap();
    assert_eq!(locator.kind(), LocatorKind::NativePath);
    assert_eq!(locator.display(), path.to_string_lossy());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(locator.encoding(), LocatorEncoding::UnixBytes);
        assert_eq!(locator.raw_bytes(), path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let bytes: Vec<u8> = path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(locator.encoding(), LocatorEncoding::WindowsUtf16Le);
        assert_eq!(locator.raw_bytes(), bytes);
    }
    assert!(locator.validate_native_path().is_ok());
    assert_eq!(locator.to_native_path().unwrap(), path);
}

#[test]
fn qualified_locator_uri_uses_utf8_identity_and_is_not_native() {
    let uri = "content://docs/中文/opaque%2Fidentity";
    let locator = QualifiedLocator::from_document_uri(uri).unwrap();
    assert_eq!(locator.kind(), LocatorKind::DocumentUri);
    assert_eq!(locator.encoding(), LocatorEncoding::Utf8Uri);
    assert_eq!(locator.raw_bytes(), uri.as_bytes());
    assert_eq!(locator.display(), uri);
    assert_eq!(
        locator.validate_native_path().unwrap_err(),
        locator.to_native_path().unwrap_err()
    );
    assert!(
        QualifiedLocator::from_parts(
            LocatorKind::NativePath,
            LocatorEncoding::Utf8Uri,
            uri.as_bytes().to_vec(),
            uri.into()
        )
        .is_err()
    );
    assert!(
        QualifiedLocator::from_parts(
            LocatorKind::DocumentUri,
            LocatorEncoding::UnixBytes,
            uri.as_bytes().to_vec(),
            uri.into()
        )
        .is_err()
    );
}

#[test]
fn qualified_locator_rejects_invalid_payload_without_display_fallback() {
    for (kind, encoding, raw, display) in [
        (
            LocatorKind::NativePath,
            LocatorEncoding::UnixBytes,
            vec![],
            "",
        ),
        (
            LocatorKind::NativePath,
            LocatorEncoding::UnixBytes,
            b"a\0b".to_vec(),
            "a\0b",
        ),
        (
            LocatorKind::NativePath,
            LocatorEncoding::UnixBytes,
            b"actual".to_vec(),
            "different",
        ),
        (
            LocatorKind::NativePath,
            LocatorEncoding::WindowsUtf16Le,
            vec![b'a'],
            "a",
        ),
        (
            LocatorKind::NativePath,
            LocatorEncoding::WindowsUtf16Le,
            vec![b'a', 0, 0, 0],
            "a\0",
        ),
        (
            LocatorKind::DocumentUri,
            LocatorEncoding::Utf8Uri,
            vec![255],
            "�",
        ),
        (
            LocatorKind::DocumentUri,
            LocatorEncoding::Utf8Uri,
            b"a\0b".to_vec(),
            "a\0b",
        ),
        (
            LocatorKind::DocumentUri,
            LocatorEncoding::Utf8Uri,
            b"uri:a".to_vec(),
            "uri:b",
        ),
    ] {
        assert!(
            QualifiedLocator::from_parts(kind, encoding, raw, display.into()).is_err(),
            "accepted {encoding:?}/{display:?}"
        );
    }
    assert!(QualifiedLocator::from_document_uri("").is_err());
    assert!(QualifiedLocator::from_native_path(std::path::Path::new("")).is_err());
}

#[test]
fn qualified_locator_keeps_same_display_distinct_raw_identities() {
    let left = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::UnixBytes,
        vec![b'f', 255],
        "f�".into(),
    )
    .unwrap();
    let right = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::UnixBytes,
        vec![b'f', 254],
        "f�".into(),
    )
    .unwrap();
    assert_eq!(left.display(), right.display());
    assert_ne!(left.raw_bytes(), right.raw_bytes());
    assert_ne!(left, right);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            left.to_native_path().unwrap().as_os_str().as_bytes(),
            &[b'f', 255]
        );
        assert_eq!(
            right.to_native_path().unwrap().as_os_str().as_bytes(),
            &[b'f', 254]
        );
    }
}

#[test]
fn qualified_locator_validates_foreign_bytes_but_refuses_native_conversion() {
    let unix = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::UnixBytes,
        b"/a".to_vec(),
        "/a".into(),
    )
    .unwrap();
    let windows = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::WindowsUtf16Le,
        vec![b'C', 0, b':', 0, b'\\', 0, b'a', 0],
        "C:\\a".into(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        assert!(unix.to_native_path().is_ok());
        assert!(windows.to_native_path().is_err());
    }
    #[cfg(windows)]
    {
        assert!(windows.to_native_path().is_ok());
        assert!(unix.to_native_path().is_err());
    }
    #[cfg(not(any(unix, windows)))]
    {
        assert!(unix.to_native_path().is_err());
        assert!(windows.to_native_path().is_err());
    }
}

#[test]
fn qualified_locator_windows_unpaired_surrogate_is_preserved() {
    let raw = vec![b'C', 0, b':', 0, b'\\', 0, 0, 0xd8];
    let locator = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::WindowsUtf16Le,
        raw.clone(),
        "C:\\�".into(),
    )
    .unwrap();
    assert_eq!(locator.raw_bytes(), raw);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let path = locator.to_native_path().unwrap();
        assert_eq!(
            path.as_os_str().encode_wide().collect::<Vec<_>>(),
            vec![67, 58, 92, 0xd800]
        );
        assert_eq!(QualifiedLocator::from_native_path(&path).unwrap(), locator);
    }
}

#[test]
fn qualified_locator_native_conversion_rechecks_public_field_integrity() {
    let mut locator = QualifiedLocator::from_native_path(std::path::Path::new("original")).unwrap();
    locator.display = "forged".into();
    assert_eq!(
        locator.validate_native_path().unwrap_err(),
        locator.to_native_path().unwrap_err()
    );
    locator.display = "original".into();
    locator.raw.clear();
    assert_eq!(
        locator.validate_native_path().unwrap_err(),
        locator.to_native_path().unwrap_err()
    );
}

#[test]
fn qualified_locator_borrowed_unix_validation_matches_standard_lossy_rendering() {
    let mut cases = vec![
        vec![0xe2, 0x82],
        vec![0xf0, 0x90, 0x80],
        vec![0xed, 0xa0, 0x80],
        vec![b'a', 0xff, b'b', 0xfe],
        vec![0xf0, 0x28, 0x8c, 0xbc],
        "a�中文🙂".as_bytes().to_vec(),
    ];
    cases.extend((1..=255).map(|byte| vec![byte]));
    for raw in cases {
        let display = String::from_utf8_lossy(&raw).into_owned();
        let mut locator = QualifiedLocator::from_parts(
            LocatorKind::NativePath,
            LocatorEncoding::UnixBytes,
            raw,
            display,
        )
        .unwrap();
        assert!(locator.validate().is_ok());
        locator.display.push('x');
        assert!(locator.validate().is_err());
    }
}

#[test]
fn qualified_locator_borrowed_utf16_validation_matches_standard_lossy_rendering() {
    for units in [
        vec![0xd800],
        vec![0xdc00],
        vec![0xd800, 0xd800],
        vec![0xdc00, 0xd800],
        vec![0xd83d, 0xde00],
        vec![0xd800, 65],
        vec![65, 0xdc00],
    ] {
        let raw = units
            .iter()
            .flat_map(|unit: &u16| unit.to_le_bytes())
            .collect();
        let display = String::from_utf16_lossy(&units);
        let mut locator = QualifiedLocator::from_parts(
            LocatorKind::NativePath,
            LocatorEncoding::WindowsUtf16Le,
            raw,
            display,
        )
        .unwrap();
        assert!(locator.validate().is_ok());
        locator.display.push('x');
        assert!(locator.validate().is_err());
    }
}

#[test]
#[cfg(any(unix, windows))]
fn qualified_locator_foreign_encoding_security_baseline() {
    // 沿用 RED 的同一 payload：两平台都能构造路径的非 NUL 字节必须按来源编码拒绝。
    #[cfg(unix)]
    let (encoding, display) = (
        LocatorEncoding::WindowsUtf16Le,
        String::from_utf16_lossy(&[0x6261, 0x6463]),
    );
    #[cfg(windows)]
    let (encoding, display) = (LocatorEncoding::UnixBytes, "abcd".to_owned());
    let foreign =
        QualifiedLocator::from_parts(LocatorKind::NativePath, encoding, b"abcd".to_vec(), display)
            .unwrap();
    assert!(foreign.validate().is_ok());
    assert_eq!(
        foreign.validate_native_path().unwrap_err(),
        foreign.to_native_path().unwrap_err()
    );
}
