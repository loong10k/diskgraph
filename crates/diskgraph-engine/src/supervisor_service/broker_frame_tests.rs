use super::broker_frame::{decode, header};

#[test]
fn original_broker_frames_keep_bounded_stdout_stderr_and_full_status() {
    for kind in [1, 2] {
        for length in [1, 4096] {
            assert_eq!(
                decode(header(kind, length).unwrap()).unwrap(),
                (kind, length)
            );
        }
    }
    assert_eq!(decode(header(3, 4).unwrap()).unwrap(), (3, 4));
}

#[test]
fn broker_frame_rejects_unknown_kind_and_lengths_before_body_allocation() {
    for (kind, length) in [
        (0, 4),
        (4, 4),
        (1, 0),
        (2, 4097),
        (3, 1),
        (3, 4096),
        (1, usize::MAX),
    ] {
        assert!(
            header(kind, length).is_err(),
            "accepted kind={kind} length={length}"
        );
    }
    assert!(decode([1, 255, 255, 255, 255]).is_err());
}
