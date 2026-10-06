use crate::EngineError;
use crate::macos_load_policy::MacosLoadPolicy;
use std::time::{Duration, Instant};

fn image(commands: Vec<Vec<u8>>) -> Vec<u8> {
    let cpu = if cfg!(target_arch = "aarch64") {
        0x0100000c_u32
    } else {
        0x01000007_u32
    };
    let size: usize = commands.iter().map(Vec::len).sum();
    let mut bytes = Vec::new();
    for word in [
        0xfeedfacf,
        cpu,
        0,
        2,
        commands.len() as u32,
        size as u32,
        0x80,
        0,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    for command in commands {
        bytes.extend(command);
    }
    bytes
}
fn name_command(kind: u32, name: &[u8]) -> Vec<u8> {
    let header = if kind == 0xc { 24 } else { 12 };
    let size = (header + name.len() + 1 + 7) & !7;
    let mut bytes = vec![0; size];
    bytes[..4].copy_from_slice(&kind.to_le_bytes());
    bytes[4..8].copy_from_slice(&(size as u32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(header as u32).to_le_bytes());
    bytes[header..header + name.len()].copy_from_slice(name);
    bytes
}
fn permitted() -> Vec<Vec<u8>> {
    vec![
        name_command(0xe, b"/usr/lib/dyld"),
        name_command(0xc, b"/usr/lib/libSystem.B.dylib"),
    ]
}
fn validate(bytes: &[u8]) -> Result<(), EngineError> {
    let mut file = tempfile::tempfile().unwrap();
    std::io::Write::write_all(&mut file, bytes).unwrap();
    MacosLoadPolicy::validate(
        &file,
        bytes.len() as u64,
        Instant::now() + Duration::from_secs(10),
        &mut || Ok(()),
    )
}
#[test]
fn fixed_absolute_system_dependencies_are_accepted() {
    validate(&image(permitted())).unwrap();
}
#[test]
fn embedded_environment_runpaths_and_alternate_loaders_are_rejected() {
    for command in [
        name_command(0x27, b"DYLD_INSERT_LIBRARIES=/tmp/evil"),
        name_command(0x8000001c, b"/tmp"),
        name_command(0xe, b"/tmp/dyld"),
    ] {
        let mut commands = permitted();
        commands.push(command);
        assert!(validate(&image(commands)).is_err());
    }
}
#[test]
fn variable_and_unapproved_dependencies_are_rejected() {
    for name in [
        b"@rpath/evil.dylib".as_slice(),
        b"@loader_path/evil",
        b"/usr/lib/../local/lib/evil",
        b"/usr/local/lib/evil",
        b"/usr/lib/evil.dylib",
    ] {
        let mut commands = permitted();
        commands.push(name_command(0xc, name));
        assert!(validate(&image(commands)).is_err());
    }
}
#[test]
fn malformed_command_counts_sizes_offsets_and_architectures_are_rejected() {
    let original = image(permitted());
    for (offset, value) in [
        (0, 0xcafebabe_u32),
        (4, 7),
        (12, 6),
        (16, 4097),
        (20, 1048577),
        (24, 0x180),
        (36, 7),
        (40, u32::MAX),
    ] {
        let mut bytes = original.clone();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(validate(&bytes).is_err(), "offset {offset}");
    }
    assert!(validate(&original[..original.len() - 1]).is_err());
}
#[test]
fn original_checkpoint_error_is_preserved_before_image_read() {
    let file = tempfile::tempfile().unwrap();
    assert!(matches!(
        MacosLoadPolicy::validate(
            &file,
            32,
            Instant::now() + Duration::from_secs(10),
            &mut || Err(EngineError::Poisoned)
        ),
        Err(EngineError::Poisoned)
    ));
}
#[test]
fn actual_cargo_macho_is_checked_from_original_handle() {
    let artifact = std::env::var_os("DISKGRAPH_MACOS_LOAD_FIXTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap());
    let file = std::fs::File::open(artifact).unwrap();
    MacosLoadPolicy::validate(
        &file,
        file.metadata().unwrap().len(),
        Instant::now() + Duration::from_secs(10),
        &mut || Ok(()),
    )
    .unwrap();
}

#[test]
fn alternate_dylib_use_encoding_cannot_smuggle_weak_or_reexport_modes() {
    for flags in [0_u32, 1, 2, 4, 8, 15, u32::MAX] {
        let library = b"/usr/lib/libSystem.B.dylib";
        let size = (28 + library.len() + 1 + 7) & !7;
        let mut command = vec![0_u8; size];
        command[..4].copy_from_slice(&0xc_u32.to_le_bytes());
        command[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        command[8..12].copy_from_slice(&28_u32.to_le_bytes());
        command[12..16].copy_from_slice(&0x1a741800_u32.to_le_bytes());
        command[24..28].copy_from_slice(&flags.to_le_bytes());
        command[28..28 + library.len()].copy_from_slice(library);
        let mut commands = permitted();
        commands.push(command);
        assert!(
            validate(&image(commands)).is_err(),
            "alternative flags {flags}"
        );
    }
}

#[test]
fn command_reads_preserve_shared_file_position_and_original_midread_cancellation() {
    use std::io::{Seek, SeekFrom, Write};
    let bytes = image(permitted());
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&bytes).unwrap();
    file.seek(SeekFrom::Start(3)).unwrap();
    MacosLoadPolicy::validate(
        &file,
        bytes.len() as u64,
        Instant::now() + Duration::from_secs(10),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(file.stream_position().unwrap(), 3);
    let mut checks = 0;
    let result = MacosLoadPolicy::validate(
        &file,
        bytes.len() as u64,
        Instant::now() + Duration::from_secs(10),
        &mut || {
            checks += 1;
            if checks == 5 {
                Err(EngineError::Poisoned)
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(EngineError::Poisoned)));
    assert_eq!(file.stream_position().unwrap(), 3);
}
