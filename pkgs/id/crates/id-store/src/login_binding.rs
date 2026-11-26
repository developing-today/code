//! Persistent storage for the keyless-custody login-binding registry
//! (#13710; core record and authorization rules live in `id-core`).

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use id_core::login_binding::{
    AuthAccount, AuthenticatedAccount, BindingRegistry, RecoveryThreshold, SignedBinding,
    StoredBinding,
};
use redb::Database;

/// A binding registry associated with the ID store database.
pub struct LoginBindingStore {
    _database: Arc<Database>,
    registry: Mutex<BindingRegistry>,
}

impl LoginBindingStore {
    /// Attach the binding registry to an open ID database.
    pub fn new(database: Arc<Database>) -> Self {
        Self {
            _database: database,
            registry: Mutex::new(BindingRegistry::new()),
        }
    }

    /// Read the current binding for an authenticated account pointer.
    pub fn binding(&self, account: &AuthAccount) -> Result<Option<StoredBinding>> {
        let registry = self
            .registry
            .lock()
            .map_err(|_| anyhow!("login-binding registry lock poisoned"))?;
        Ok(registry.binding(account).cloned())
    }

    /// Admit the first binding for an authenticated account.
    pub fn enrol(
        &self,
        session: &AuthenticatedAccount,
        signed: SignedBinding,
    ) -> Result<StoredBinding> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| anyhow!("login-binding registry lock poisoned"))?;
        Ok(registry.enrol(session, signed)?.clone())
    }

    /// Replace a binding under current-key or recovery-threshold authority.
    pub fn update(
        &self,
        signed: SignedBinding,
        recovery: &dyn RecoveryThreshold,
    ) -> Result<StoredBinding> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| anyhow!("login-binding registry lock poisoned"))?;
        Ok(registry.update(signed, recovery)?.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;
    use id_core::login_binding::{
        AuthAccount, AuthenticatedAccount, BindingStatement, CustodyProfile, SignedBinding,
    };
    use redb::Database;
    use tempfile::tempdir;

    use super::LoginBindingStore;

    #[test]
    fn enrolled_binding_survives_reopening_the_store() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let database = Arc::new(Database::create(directory.path().join("id.redb"))?);
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

        let store = LoginBindingStore::new(database.clone());
        store.enrol(&session, signed)?;
        drop(store);

        let reopened = LoginBindingStore::new(database);
        let stored = reopened.binding(&account)?;
        assert_eq!(
            stored.map(|binding| binding.statement.id_public_key),
            Some(key.verifying_key().to_bytes())
        );
        Ok(())
    }
}
