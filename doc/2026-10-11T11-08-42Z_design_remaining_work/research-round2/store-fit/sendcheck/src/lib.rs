//! Compile-time checks for a redb-backed p2panda-store `Transaction` impl: can the store hold
//! a `WriteTransaction` behind a tokio Mutex across awaits, and are the resulting futures Send?
use std::future::Future;
use std::sync::Arc;

use redb::{Database, WriteTransaction};
use tokio::sync::{Mutex, Semaphore};

fn needs_send<T: Send>() {}
fn needs_sync<T: Sync>() {}
fn needs_send_future<F: Future + Send>(_: F) {}

pub struct RedbStore {
    db: Arc<Database>,
    permit: Arc<Semaphore>,
    txn: Arc<Mutex<Option<WriteTransaction>>>,
}

pub async fn begin_hold_commit(store: Arc<RedbStore>) {
    let _permit = store.permit.clone().acquire_owned().await.unwrap();
    let db = store.db.clone();
    let txn = tokio::task::spawn_blocking(move || db.begin_write())
        .await
        .unwrap()
        .unwrap();
    store.txn.lock().await.replace(txn);
    tokio::task::yield_now().await;
    let txn = store.txn.lock().await.take().unwrap();
    txn.commit().unwrap();
}

pub fn assertions(store: Arc<RedbStore>) {
    needs_send::<WriteTransaction>();
    needs_send::<Database>();
    needs_sync::<Database>();
    needs_send::<RedbStore>();
    needs_sync::<RedbStore>();
    needs_send_future(begin_hold_commit(store));
    needs_send::<redb::Range<'static, &'static [u8], &'static [u8]>>();
}
