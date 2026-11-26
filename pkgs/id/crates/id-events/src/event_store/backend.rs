//! Transaction ownership for the shared event-store semantics.
//!
//! A backend holds one read snapshot or one write transaction for the full
//! operation. The event-store helpers see only owned rows; neither redb guards
//! nor the tenant Turso session escape their backend.

use std::fmt;
use std::sync::{Mutex, MutexGuard};

use id_store::event_rows::{EventWriteTransaction, RowRead, RowWrite, TenantWrite};
use id_store::event_types::StoreError;
use id_store::turso::{PipelineTransport, TenantStore};
use redb::{Database, ReadableDatabase};

/// Runs event-store operations inside backend-owned transactions.
///
/// `with_read` holds a consistent read view across the entire operation.
/// `with_write` commits the complete operation when its change bit is true and
/// aborts it otherwise. Returning an error leaves the backend responsible for
/// rollback and, for an uncertain tenant session, retirement.
pub trait EventStoreStorage: fmt::Debug + Send + Sync + 'static {
    /// Runs `operation` against one backend-owned read snapshot.
    fn with_read<T>(
        &self,
        operation: impl FnOnce(&mut dyn RowRead) -> Result<T, StoreError>,
    ) -> Result<T, StoreError>;

    /// Runs `operation` against one backend-owned write transaction.
    fn with_write<T>(
        &self,
        operation: impl FnOnce(&mut dyn RowWrite) -> Result<(T, bool), StoreError>,
    ) -> Result<T, StoreError>;
}

impl EventStoreStorage for Database {
    fn with_read<T>(
        &self,
        operation: impl FnOnce(&mut dyn RowRead) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut txn = self.begin_read()?;
        operation(&mut txn)
    }

    fn with_write<T>(
        &self,
        operation: impl FnOnce(&mut dyn RowWrite) -> Result<(T, bool), StoreError>,
    ) -> Result<T, StoreError> {
        let mut txn = self.begin_write()?;
        match operation(&mut txn) {
            Ok((value, changed)) => {
                if changed {
                    EventWriteTransaction::commit(txn)?;
                } else {
                    EventWriteTransaction::abort(txn)?;
                }
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }
}

/// Tenant-store backend for the same event-store row semantics.
///
/// The mutex serializes access to the stateful Turso baton. It stays held for
/// the whole transaction closure so multi-table reads share a snapshot and no
/// second local operation can interleave with a write.
#[derive(Debug)]
pub struct TenantBackend<T: PipelineTransport> {
    store: Mutex<TenantStore<T>>,
}

impl<T: PipelineTransport> TenantBackend<T> {
    /// Opens the tenant row store and owns its single baton session.
    pub fn open(transport: T) -> Result<Self, StoreError> {
        Ok(Self {
            store: Mutex::new(TenantStore::open(transport)?),
        })
    }

    fn lock_store(&self) -> MutexGuard<'_, TenantStore<T>> {
        // A panic unwinds through TenantWrite, which rolls back or closes an
        // uncertain session before the mutex is poisoned. Recovering the guard
        // is therefore safe; a retired session returns its typed closed error.
        self.store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<T: PipelineTransport> EventStoreStorage for TenantBackend<T> {
    fn with_read<R>(
        &self,
        operation: impl FnOnce(&mut dyn RowRead) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let mut store = self.lock_store();
        let mut txn = TenantWrite::begin(&mut store)?;
        let mut rows = &mut txn;
        let value = operation(&mut rows)?;
        txn.rollback()?;
        Ok(value)
    }

    fn with_write<R>(
        &self,
        operation: impl FnOnce(&mut dyn RowWrite) -> Result<(R, bool), StoreError>,
    ) -> Result<R, StoreError> {
        let mut store = self.lock_store();
        let mut txn = TenantWrite::begin(&mut store)?;
        let mut rows = &mut txn;
        let (value, changed) = operation(&mut rows)?;
        if changed {
            txn.commit()?;
        } else {
            txn.rollback()?;
        }
        Ok(value)
    }
}
