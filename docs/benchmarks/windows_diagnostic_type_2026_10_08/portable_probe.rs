#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowsFileState {
    pub(crate) volume: u64,
    id: [u8; 16],
    pub(crate) len: u64,
    creation: i64,
    modified: i64,
    changed: i64,
    attributes: u32,
    pub(crate) directory: bool,
    delete_pending: bool,
}

impl WindowsFileState {
    #[cfg(test)]
    pub(crate) fn changed_mask(&self, other: &Self) -> u16 {
        u16::from(self.volume != other.volume)
            | (u16::from(self.id != other.id) << 1)
            | (u16::from(self.len != other.len) << 2)
            | (u16::from(self.creation != other.creation) << 3)
            | (u16::from(self.modified != other.modified) << 4)
            | (u16::from(self.changed != other.changed) << 5)
            | (u16::from(self.attributes != other.attributes) << 6)
            | (u16::from(self.directory != other.directory) << 7)
            | (u16::from(self.delete_pending != other.delete_pending) << 8)
    }

}
#[test]
fn exact_diagnostic_type_contract() {
 let before = WindowsFileState { volume:0,id:[0;16],len:0,creation:0,modified:0,changed:0,attributes:0,directory:true,delete_pending:false };
 let mut after = before.clone(); after.changed=1;
 let version_mask = std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0));
 let recorded_mask=version_mask.clone();
                recorded_mask.store(
                    before.changed_mask(&after),
                    std::sync::atomic::Ordering::Relaxed,
                );
 assert_eq!(version_mask.load(std::sync::atomic::Ordering::Relaxed),0x020);
}
