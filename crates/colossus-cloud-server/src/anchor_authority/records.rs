use super::*;

pub(super) trait AnchorRecords: Send + Sync {
    fn read(&self, namespace: &str) -> CloudResult<ChatRecord>;
    fn write(&self, namespace: &str, expected_revision: u64, ciphertext: &[u8]) -> CloudResult<()>;
}

impl AnchorRecords for ChatSqlStore {
    fn read(&self, namespace: &str) -> CloudResult<ChatRecord> {
        self.read_record(namespace, RECORD)
    }
    fn write(&self, namespace: &str, expected_revision: u64, ciphertext: &[u8]) -> CloudResult<()> {
        self.write_records(&[ChatRecordWrite {
            namespace,
            record_id: RECORD,
            expected_revision,
            ciphertext: Some(ciphertext),
            ttl_seconds: None,
        }])
        .map(|_| ())
    }
}
