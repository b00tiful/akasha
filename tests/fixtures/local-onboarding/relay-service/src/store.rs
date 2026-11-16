/// Store is process-local memory: append adds encoded records; there is no disk persistence.
#[derive(Default)]
pub struct Store {
    pub records: Vec<Vec<u8>>,
}
impl Store {
    pub fn append(&mut self, bytes: Vec<u8>) {
        self.records.push(bytes);
    }
}
