//! Persistent storage for the keyless-custody login-binding registry
//! (#13710; core record and authorization rules live in `id-core`).

use std::sync::Arc;

use anyhow::Result;
use id_core::login_binding::{
    AuthAccount, AuthenticatedAccount, BindingRegistry, RecoveryThreshold, SignedBinding,
    StoredBinding,
};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const HISTORY: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("login_binding_history_v1");
const HISTORY_KEY: &str = "accepted-operations";

/// A binding registry associated with the ID store database.
pub struct LoginBindingStore {
    database: Arc<Database>,
}

impl LoginBindingStore {
    /// Attach the binding registry to an open ID database.
    pub fn new(database: Arc<Database>) -> Result<Self> {
        let transaction = database.begin_write()?;
        transaction.open_table(HISTORY)?;
        transaction.commit()?;
        Ok(Self { database })
    }

    /// Read the current binding for an authenticated account pointer.
    pub fn binding(
        &self,
        account: &AuthAccount,
        recovery: &dyn RecoveryThreshold,
    ) -> Result<Option<StoredBinding>> {
        let transaction = self.database.begin_read()?;
        let table = transaction.open_table(HISTORY)?;
        let bytes = table.get(HISTORY_KEY)?.map(|entry| entry.value().to_vec());
        let registry = restore(bytes, recovery)?;
        Ok(registry.binding(account).cloned())
    }

    /// Admit the first binding for an authenticated account.
    pub fn enrol(
        &self,
        session: &AuthenticatedAccount,
        signed: SignedBinding,
        recovery: &dyn RecoveryThreshold,
    ) -> Result<StoredBinding> {
        let transaction = self.database.begin_write()?;
        let mut table = transaction.open_table(HISTORY)?;
        let bytes = table.get(HISTORY_KEY)?.map(|entry| entry.value().to_vec());
        let mut registry = restore(bytes, recovery)?;
        let stored = registry.enrol(session, signed)?.clone();
        let history = registry.history_bytes()?;
        table.insert(HISTORY_KEY, history.as_slice())?;
        drop(table);
        transaction.commit()?;
        Ok(stored)
    }

    /// Replace a binding under current-key or recovery-threshold authority.
    pub fn update(
        &self,
        signed: SignedBinding,
        recovery: &dyn RecoveryThreshold,
    ) -> Result<StoredBinding> {
        let transaction = self.database.begin_write()?;
        let mut table = transaction.open_table(HISTORY)?;
        let bytes = table.get(HISTORY_KEY)?.map(|entry| entry.value().to_vec());
        let mut registry = restore(bytes, recovery)?;
        let stored = registry.update(signed, recovery)?.clone();
        let history = registry.history_bytes()?;
        table.insert(HISTORY_KEY, history.as_slice())?;
        drop(table);
        transaction.commit()?;
        Ok(stored)
    }
}

fn restore(history: Option<Vec<u8>>, recovery: &dyn RecoveryThreshold) -> Result<BindingRegistry> {
    match history {
        Some(bytes) => Ok(BindingRegistry::from_history_bytes(&bytes, recovery)?),
        None => Ok(BindingRegistry::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;
    use id_core::login_binding::{
        AuthAccount, AuthenticatedAccount, BindingStatement, CustodyProfile, NoRecoveryThreshold,
        SignedBinding,
    };
    use redb::Database;
    use tempfile::tempdir;

    use super::LoginBindingStore;

    #[test]
    fn enrolled_and_rotated_binding_survive_reopening_the_store() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let database_path = directory.path().join("id.redb");
        let database = Arc::new(Database::create(&database_path)?);
        let recovery = NoRecoveryThreshold;
        let key = SigningKey::from_bytes(&[9; 32]);
        let account = AuthAccount::opaque("alice").expect("valid account");
        let session = AuthenticatedAccount::from_verified_login(account.clone());
        let signed = SignedBinding::enrol(
            BindingStatement {
                id_public_key: key.verifying_key().to_bytes(),
                account: account.clone(),
                custody: CustodyProfile::UserHeld,
                recovery_commitment: [4; 32],
                generation: 1,
                prev: None,
            },
            &key,
        )?;

        let store = LoginBindingStore::new(database.clone())?;
        let first = store.enrol(&session, signed, &recovery)?;
        let next_key = SigningKey::from_bytes(&[10; 32]);
        let next_statement = BindingStatement {
            id_public_key: next_key.verifying_key().to_bytes(),
            generation: 2,
            prev: Some(first.statement_hash),
            ..first.statement
        };
        store.update(
            SignedBinding::update(next_statement, &next_key, &key)?,
            &recovery,
        )?;
        drop(store);
        drop(database);

        let reopened = LoginBindingStore::new(Arc::new(Database::open(&database_path)?))?;
        let stored = reopened.binding(&account, &recovery)?;
        let stored = stored.expect("persisted binding");
        assert_eq!(
            stored.statement.id_public_key,
            next_key.verifying_key().to_bytes()
        );
        assert_eq!(stored.statement.generation, 2);
        Ok(())
    }
}
