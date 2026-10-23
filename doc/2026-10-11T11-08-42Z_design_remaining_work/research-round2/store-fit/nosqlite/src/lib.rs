//! Can a type in our crate implement p2panda-store's traits with default-features = false?
use std::collections::BTreeMap;

use futures_util::stream::{self, BoxStream};
use p2panda_core::{AnyOperation, Hash, SeqNum, VerifyingKey};
use p2panda_store::Transaction;
use p2panda_store::logs::{LogEntry, LogStore};

pub struct RedbStore;

#[derive(Debug)]
pub struct StoreError;

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "store error")
    }
}

impl std::error::Error for StoreError {}

impl Transaction for RedbStore {
    type Error = StoreError;
    type Permit = ();

    async fn begin(&self) -> Result<(), StoreError> {
        Ok(())
    }

    async fn rollback(&self, _permit: ()) -> Result<(), StoreError> {
        Ok(())
    }

    async fn commit(&self, _permit: ()) -> Result<(), StoreError> {
        Ok(())
    }
}

impl<L> LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash> for RedbStore
where
    L: p2panda_core::LogId + Send + Sync + 'static,
{
    type Error = StoreError;

    async fn get_latest_entry(&self, _author: &VerifyingKey, _log_id: &L) -> Result<Option<AnyOperation>, StoreError> {
        Ok(None)
    }

    async fn get_latest_entry_tx(&self, _author: &VerifyingKey, _log_id: &L) -> Result<Option<AnyOperation>, StoreError> {
        Ok(None)
    }

    async fn get_log_heights(&self, _author: &VerifyingKey, _logs: &[L]) -> Result<Option<BTreeMap<L, SeqNum>>, StoreError> {
        Ok(None)
    }

    async fn get_log_size(&self, _author: &VerifyingKey, _log_id: &L, _after: Option<SeqNum>, _until: Option<SeqNum>) -> Result<Option<(u32, u32)>, StoreError> {
        Ok(None)
    }

    fn log_entries(&self, _author: &VerifyingKey, _log_id: &L, _after: Option<SeqNum>, _until: Option<SeqNum>) -> Result<BoxStream<'static, Result<LogEntry<AnyOperation, L>, StoreError>>, StoreError> {
        Ok(Box::pin(stream::empty()))
    }

    async fn prune_entries(&self, _author: &VerifyingKey, _log_id: &L, _until: &SeqNum) -> Result<u64, StoreError> {
        Ok(0)
    }
}

// Ungated trait modules: importing them checks that they compile with default features off.
#[allow(unused_imports)]
use p2panda_store::cursors::CursorStore as _CursorStoreCheck;
#[allow(unused_imports)]
use p2panda_store::operations::OperationStore as _OperationStoreCheck;
#[allow(unused_imports)]
use p2panda_store::orderer::OrdererStore as _OrdererStoreCheck;
#[allow(unused_imports)]
use p2panda_store::topics::TopicStore as _TopicStoreCheck;
