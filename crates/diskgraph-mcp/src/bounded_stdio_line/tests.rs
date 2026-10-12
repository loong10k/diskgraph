use super::BoundedStdioLine;
use std::io::{self, BufReader, Cursor};
#[test]
fn overlong_unterminated_input_is_refused_before_growth_or_dispatch() {
    let source = vec![b'x'; (1 << 20) + 4096];
    let mut input = BufReader::with_capacity(4096, Cursor::new(source));
    let mut reader = BoundedStdioLine::new();
    assert_eq!(
        reader.read(&mut input).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(reader.bytes.len() <= 1 << 20);
    assert!(reader.bytes.capacity() <= 1 << 20);
}
#[test]
fn complete_lines_keep_original_crlf_unicode_and_eof_semantics() {
    let mut input = Cursor::new("İSTANBUL\r\n\nlast\r".as_bytes());
    let mut reader = BoundedStdioLine::new();
    assert_eq!(
        reader.read(&mut input).unwrap().as_deref(),
        Some("İSTANBUL")
    );
    assert_eq!(reader.read(&mut input).unwrap().as_deref(), Some(""));
    assert_eq!(reader.read(&mut input).unwrap().as_deref(), Some("last\r"));
    assert!(reader.read(&mut input).unwrap().is_none());
    assert!(reader.read(&mut Cursor::new([0xff, b'\n'])).is_err());
}
