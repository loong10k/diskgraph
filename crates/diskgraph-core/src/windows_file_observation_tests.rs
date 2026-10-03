use crate::{WindowsFileObservation, WindowsObservationGap, WindowsTreeAlignment};

fn observed() -> WindowsFileObservation {
    WindowsFileObservation {
        volume: u64::MAX,
        file_id: [0x8f; 16],
        length: i64::MAX as u64,
        creation_time: i64::MIN,
        last_write_time: 13_400_000_000_000_001,
        change_time: i64::MAX,
        attributes: 0x20,
        directory: false,
        delete_pending: false,
        capture_started_unix_ms: u64::MAX - 1,
        capture_finished_unix_ms: u64::MAX,
        tree_alignment: WindowsTreeAlignment::Unverified,
    }
}

#[test]
fn full_width_identity_and_native_ticks_survive_codec() {
    let observation = observed();
    let bytes = observation.encode().unwrap();
    assert_eq!(
        WindowsFileObservation::FORMAT_LABEL,
        "windows_file_observation_v1"
    );
    assert_eq!(WindowsFileObservation::ENCODED_LEN, 80);
    assert_eq!(bytes.len(), 80);
    assert_eq!(WindowsFileObservation::decode(&bytes).unwrap(), observation);
    assert!(observation.legacy_identity().is_none());
    assert_eq!(observation.volume_key(), "windows-volume-ffffffffffffffff");
}

#[test]
fn codec_has_a_fixed_little_endian_layout() {
    let mut observation = observed();
    observation.volume = 0x0102_0304_0506_0708;
    observation.file_id = std::array::from_fn(|index| index as u8);
    observation.length = 9;
    observation.creation_time = -2;
    observation.last_write_time = 3;
    observation.change_time = -4;
    observation.attributes = 0x1234_5610;
    observation.directory = true;
    observation.delete_pending = true;
    observation.capture_started_unix_ms = 5;
    observation.capture_finished_unix_ms = 6;
    observation.tree_alignment = WindowsTreeAlignment::Matched;
    let bytes = observation.encode().unwrap();
    assert_eq!(bytes[0], 1);
    assert_eq!(&bytes[1..9], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(&bytes[9..25], &observation.file_id);
    assert_eq!(&bytes[25..33], &9u64.to_le_bytes());
    assert_eq!(&bytes[33..41], &(-2i64).to_le_bytes());
    assert_eq!(&bytes[41..49], &3i64.to_le_bytes());
    assert_eq!(&bytes[49..57], &(-4i64).to_le_bytes());
    assert_eq!(&bytes[57..61], &0x1234_5610u32.to_le_bytes());
    assert_eq!(&bytes[61..63], &[1, 1]);
    assert_eq!(&bytes[63..71], &5u64.to_le_bytes());
    assert_eq!(&bytes[71..79], &6u64.to_le_bytes());
    assert_eq!(bytes[79], 0);
    assert_eq!(WindowsFileObservation::decode(&bytes).unwrap(), observation);
}

#[test]
fn same_low_id_bits_do_not_collapse_distinct_full_identities() {
    let mut left = observed();
    left.file_id = [0; 16];
    left.file_id[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    let legacy = left.legacy_identity().unwrap();
    assert_eq!(legacy.file_id, u64::MAX);
    assert_eq!(legacy.volume_id, left.volume_key());
    let mut right = left.clone();
    right.file_id[15] = 1;
    assert_ne!(left, right);
    assert_ne!(left.encode().unwrap(), right.encode().unwrap());
    assert!(right.legacy_identity().is_none());
    assert_eq!(
        WindowsFileObservation::decode(&right.encode().unwrap()).unwrap(),
        right
    );
}

#[test]
fn subsecond_last_write_and_change_ticks_remain_distinct() {
    let before = observed();
    for change_field in [false, true] {
        let mut after = before.clone();
        if change_field {
            after.change_time -= 1;
        } else {
            after.last_write_time += 1;
            assert_eq!(
                before.last_write_time / 10_000_000,
                after.last_write_time / 10_000_000
            );
        }
        assert_ne!(before.encode().unwrap(), after.encode().unwrap());
        assert_eq!(
            WindowsFileObservation::decode(&after.encode().unwrap()).unwrap(),
            after
        );
    }
}

#[test]
fn codec_rejects_unknown_versions_lengths_flags_and_alignment() {
    let bytes = observed().encode().unwrap();
    assert!(WindowsFileObservation::decode(&[]).is_err());
    assert!(WindowsFileObservation::decode(&bytes[..79]).is_err());
    let mut oversized = bytes.to_vec();
    oversized.push(0);
    assert!(WindowsFileObservation::decode(&oversized).is_err());
    for (offset, value) in [(0, 0), (0, 2), (61, 2), (62, 255), (79, 2)] {
        let mut invalid = bytes;
        invalid[offset] = value;
        assert!(
            WindowsFileObservation::decode(&invalid).is_err(),
            "offset {offset}"
        );
    }
}

#[test]
fn public_field_mutation_is_revalidated_before_encoding_or_projection() {
    let mut invalid_values = Vec::new();
    let mut backwards = observed();
    backwards.capture_started_unix_ms = backwards.capture_finished_unix_ms;
    backwards.capture_finished_unix_ms -= 1;
    invalid_values.push(backwards);
    let mut excessive_length = observed();
    excessive_length.length = i64::MAX as u64 + 1;
    invalid_values.push(excessive_length);
    let mut wrong_directory = observed();
    wrong_directory.directory = true;
    invalid_values.push(wrong_directory);
    let mut wrong_attribute = observed();
    wrong_attribute.attributes |= 0x10;
    invalid_values.push(wrong_attribute);
    for mut observation in invalid_values {
        observation.file_id = [0; 16];
        assert!(observation.validate().is_err());
        assert!(observation.encode().is_err());
        assert!(observation.legacy_identity().is_none());
    }
}

#[test]
fn decode_checks_semantics_after_reading_fixed_fields() {
    let bytes = observed().encode().unwrap();
    let mut backwards = bytes;
    backwards[63..71].copy_from_slice(&u64::MAX.to_le_bytes());
    backwards[71..79].copy_from_slice(&(u64::MAX - 1).to_le_bytes());
    let mut oversized_length = bytes;
    oversized_length[25..33].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut directory_mismatch = bytes;
    directory_mismatch[61] = 1;
    for invalid in [backwards, oversized_length, directory_mismatch] {
        assert!(WindowsFileObservation::decode(&invalid).is_err());
    }
}

#[test]
fn zero_values_equal_capture_times_and_unverified_alignment_are_representable() {
    let mut observation = observed();
    observation.volume = 0;
    observation.file_id = [0; 16];
    observation.length = 0;
    observation.creation_time = 0;
    observation.last_write_time = 0;
    observation.change_time = 0;
    observation.capture_started_unix_ms = 0;
    observation.capture_finished_unix_ms = 0;
    assert_eq!(
        WindowsFileObservation::decode(&observation.encode().unwrap()).unwrap(),
        observation
    );
    assert_eq!(observation.volume_key(), "windows-volume-0000000000000000");
}

#[test]
fn gap_and_tree_alignment_labels_are_explicit_and_strict() {
    for (gap, code) in [
        (WindowsObservationGap::NotCaptured, "not_captured"),
        (WindowsObservationGap::Unsupported, "unsupported"),
        (WindowsObservationGap::CaptureFailed, "capture_failed"),
        (WindowsObservationGap::Changed, "changed"),
        (WindowsObservationGap::TreeMismatch, "tree_mismatch"),
    ] {
        assert_eq!(gap.code(), code);
        assert_eq!(WindowsObservationGap::parse(code).unwrap(), gap);
    }
    for invalid in ["", "Unknown", "changed ", "CHANGED"] {
        assert!(WindowsObservationGap::parse(invalid).is_err());
    }
    for (alignment, code) in [
        (WindowsTreeAlignment::Matched, "matched"),
        (WindowsTreeAlignment::Unverified, "unverified"),
    ] {
        assert_eq!(alignment.code(), code);
        assert_eq!(WindowsTreeAlignment::parse(code).unwrap(), alignment);
    }
    assert!(WindowsTreeAlignment::parse("unknown").is_err());
}
