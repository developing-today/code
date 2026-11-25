//! Storage-only write seam for the shared event persistence helpers.
//!
//! A handle is an already-open write transaction. Helpers neither commit nor
//! own ingest policy: the caller commits the whole batch or aborts it on error.
//! Reads return owned bytes so no backend's row guards leak into event logic.

use redb::{
    MultimapTableDefinition, ReadableMultimapTable, ReadableTable, TableDefinition,
    WriteTransaction,
};

use crate::event_types::StoreError;
use crate::turso::{PipelineTransport, TenantStore, TursoError};

/// The row operations needed by event/body/chain/fork persistence.
pub trait RowWrite {
    /// Reads one value inside this transaction.
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
    /// Replaces one value inside this transaction.
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError>;
    /// Adds a value to a multimap set inside this transaction.
    fn multi_put(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError>;
    /// Removes one value from a single-value table, if present.
    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError>;
    /// Removes one value from a multimap set, if present.
    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError>;
    /// Reads all values under one multimap key in byte order.
    fn multi_values(&mut self, table: &'static str, key: &[u8])
    -> Result<Vec<Vec<u8>>, StoreError>;
    /// Reads every row of a single-value table in key order.
    fn scan_all(&mut self, table: &'static str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError>;
}

impl<R: RowWrite + ?Sized> RowWrite for &mut R {
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        (**self).get(table, key)
    }

    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        (**self).put(table, key, value)
    }

    fn multi_put(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        (**self).multi_put(table, key, value)
    }

    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        (**self).remove(table, key)
    }

    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        (**self).multi_remove(table, key, value)
    }

    fn multi_values(
        &mut self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        (**self).multi_values(table, key)
    }

    fn scan_all(&mut self, table: &'static str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        (**self).scan_all(table)
    }
}

/// Native adapter: redb's transaction already has interior-mutating table opens.
impl RowWrite for WriteTransaction {
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let table = self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?;
        Ok(table.get(key)?.map(|row| row.value().to_vec()))
    }

    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?
            .insert(key, value)?;
        Ok(())
    }

    fn multi_put(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.open_multimap_table(MultimapTableDefinition::<&[u8], &[u8]>::new(table))?
            .insert(key, value)?;
        Ok(())
    }

    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?
            .remove(key)?;
        Ok(())
    }

    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.open_multimap_table(MultimapTableDefinition::<&[u8], &[u8]>::new(table))?
            .remove(key, value)?;
        Ok(())
    }

    fn multi_values(
        &mut self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let table =
            self.open_multimap_table(MultimapTableDefinition::<&[u8], &[u8]>::new(table))?;
        table
            .get(key)?
            .map(|value| value.map(|row| row.value().to_vec()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn scan_all(&mut self, table: &'static str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        let table = self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?;
        table
            .iter()?
            .map(|row| {
                let (key, value) = row?;
                Ok((key.value().to_vec(), value.value().to_vec()))
            })
            .collect()
    }
}

/// Compatibility adapter for helpers that borrow an already-open native transaction.
impl RowWrite for &WriteTransaction {
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let table = self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?;
        Ok(table.get(key)?.map(|row| row.value().to_vec()))
    }

    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?
            .insert(key, value)?;
        Ok(())
    }

    fn multi_put(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.open_multimap_table(MultimapTableDefinition::<&[u8], &[u8]>::new(table))?
            .insert(key, value)?;
        Ok(())
    }

    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?
            .remove(key)?;
        Ok(())
    }

    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.open_multimap_table(MultimapTableDefinition::<&[u8], &[u8]>::new(table))?
            .remove(key, value)?;
        Ok(())
    }

    fn multi_values(
        &mut self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let table =
            self.open_multimap_table(MultimapTableDefinition::<&[u8], &[u8]>::new(table))?;
        table
            .get(key)?
            .map(|value| value.map(|row| row.value().to_vec()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn scan_all(&mut self, table: &'static str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        let table = self.open_table(TableDefinition::<&[u8], &[u8]>::new(table))?;
        table
            .iter()?
            .map(|row| {
                let (key, value) = row?;
                Ok((key.value().to_vec(), value.value().to_vec()))
            })
            .collect()
    }
}

/// Tenant transaction adapter. Dropping without commit rolls back all helper
/// writes, including partial writes before a transport or storage error.
/// Only a mutable borrow implements [`RowWrite`], so a helper cannot consume
/// the guard and report success just before its implicit rollback.
///
/// ```compile_fail
/// use id_store::event_rows::{RowWrite, TenantWrite};
/// use id_store::turso::PipelineTransport;
/// fn helper(_: impl RowWrite) {}
/// fn cannot_consume<T: PipelineTransport>(txn: TenantWrite<'_, T>) {
///     helper(txn);
/// }
/// ```
#[derive(Debug)]
pub struct TenantWrite<'a, T: PipelineTransport> {
    store: &'a mut TenantStore<T>,
    finished: bool,
}

impl<'a, T: PipelineTransport> TenantWrite<'a, T> {
    /// Begins a transaction, refusing a nested transaction.
    pub fn begin(store: &'a mut TenantStore<T>) -> Result<Self, StoreError> {
        if let Err(error) = store.begin() {
            // A nested transaction is rejected locally without a request. All
            // other failures may hide a successfully executed BEGIN; no guard
            // exists yet to retire that uncertain session on drop.
            if !matches!(error, TursoError::Transaction(_)) {
                let _ = store.close();
            }
            return Err(error.into());
        }
        Ok(Self {
            store,
            finished: false,
        })
    }

    /// Commits the batch. A failed commit attempts rollback on drop.
    pub fn commit(mut self) -> Result<(), StoreError> {
        self.store.commit()?;
        self.finished = true;
        Ok(())
    }

    /// Explicit rollback, with any rollback error preserved for the caller.
    pub fn rollback(mut self) -> Result<(), StoreError> {
        self.store.rollback()?;
        self.finished = true;
        Ok(())
    }
}

impl<T: PipelineTransport> Drop for TenantWrite<'_, T> {
    fn drop(&mut self) {
        if !self.finished && self.store.rollback().is_err() {
            // Rollback could not be confirmed. Never reuse an uncertain baton:
            // close marks the connection closed even when the network is down.
            let _ = self.store.close();
        }
    }
}

impl<T: PipelineTransport> RowWrite for &mut TenantWrite<'_, T> {
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.store.get(table, key)?)
    }

    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        Ok(self.store.put(table, key, value)?)
    }

    fn multi_put(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        Ok(self.store.multi_put(table, key, value)?)
    }

    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        self.store.remove(table, key)?;
        Ok(())
    }

    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.store.multi_remove(table, key, value)?;
        Ok(())
    }

    fn multi_values(
        &mut self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        Ok(self.store.multi_values(table, key)?)
    }

    fn scan_all(&mut self, table: &'static str) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        Ok(self.store.scan(table, Vec::<u8>::new().., usize::MAX)?)
    }
}
