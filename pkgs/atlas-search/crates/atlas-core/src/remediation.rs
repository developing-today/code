use crate::filters::Sensitivity;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InventoryCounts {
    pub source: u64,
    pub destination: u64,
    pub verified: u64,
    pub missing_destination: u64,
    pub hash_mismatch: u64,
    pub missing_vector: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChunk {
    pub id: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationChunk {
    pub id: i64,
    pub content_hash: String,
    pub has_vector: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingStatus {
    Verified,
    MissingDestination,
    HashMismatch,
    MissingVector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingEntry {
    pub source_chunk_id: i64,
    pub source_content_hash: String,
    pub destination_chunk_id: Option<i64>,
    pub destination_content_hash: Option<String>,
    pub status: MappingStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InventoryReport {
    pub counts: InventoryCounts,
    pub mappings: Vec<MappingEntry>,
}

pub fn build_report(
    _source_name: &str,
    _destination_name: &str,
    _sensitivity: Sensitivity,
    _source: &[SourceChunk],
    _destination: &[DestinationChunk],
) -> InventoryReport {
    InventoryReport::default()
}
