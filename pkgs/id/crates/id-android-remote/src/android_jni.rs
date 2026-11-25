//! Small JNI surface for the Kotlin Android shell.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, LazyLock, Mutex,
    atomic::{AtomicI64, Ordering},
};
use std::thread;

use jni::JNIEnv;
use jni::objects::{JObject, JString};
use jni::sys::{jlong, jstring};
use serde::Serialize;
use tokio::sync::watch;
use zeroize::{Zeroize, Zeroizing};

use crate::{RemoteView, bind_public_client_endpoint, join_world_session, parse_endpoint_addr};

static NEXT_SESSION_ID: AtomicI64 = AtomicI64::new(1);
static SESSIONS: LazyLock<Mutex<HashMap<jlong, Arc<SessionControl>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug, Serialize)]
struct SessionSnapshot {
    status: String,
    view: Option<RemoteView>,
    error: Option<String>,
}

struct SessionControl {
    snapshot: Mutex<SessionSnapshot>,
    cancel: watch::Sender<bool>,
}

fn update(control: &SessionControl, status: &str, view: Option<RemoteView>, error: Option<String>) {
    if let Ok(mut snapshot) = control.snapshot.lock() {
        snapshot.status = status.to_owned();
        if let Some(view) = view {
            snapshot.view = Some(view);
        }
        snapshot.error = error;
    }
}

fn java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Option<String> {
    env.get_string(value).ok().map(Into::into)
}

fn spawn_session(address: String, world: String, capability: String) -> jlong {
    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    if id <= 0 {
        return 0;
    }
    let (cancel, cancel_rx) = watch::channel(false);
    let control = Arc::new(SessionControl {
        snapshot: Mutex::new(SessionSnapshot {
            status: "connecting".to_owned(),
            view: None,
            error: None,
        }),
        cancel,
    });
    let Ok(mut sessions) = SESSIONS.lock() else {
        return 0;
    };
    sessions.insert(id, Arc::clone(&control));
    drop(sessions);

    let worker_control = Arc::clone(&control);
    let spawned = thread::Builder::new()
        .name("id-android-remote-session".to_owned())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async move {
                    let endpoint = bind_public_client_endpoint().await?;
                    let addr = parse_endpoint_addr(&address)?;
                    let mut capability = Zeroizing::new(capability);
                    let joined = tokio::select! {
                        result = join_world_session(
                            &endpoint,
                            addr,
                            (!world.trim().is_empty()).then_some(world.as_str()),
                            capability.as_str(),
                        ) => Some(result),
                        _ = wait_for_cancel(cancel_rx) => None,
                    };
                    capability.zeroize();
                    let Some(joined) = joined else {
                        endpoint.close().await;
                        return Ok::<(), anyhow::Error>(());
                    };
                    let (mut session, first_view) = joined?;
                    update(&worker_control, "connected", Some(first_view), None);
                    let mut cancel_rx = worker_control.cancel.subscribe();
                    if *cancel_rx.borrow() {
                        endpoint.close().await;
                        return Ok::<(), anyhow::Error>(());
                    }
                    loop {
                        tokio::select! {
                            changed = cancel_rx.changed() => {
                                if changed.is_err() || *cancel_rx.borrow() {
                                    break;
                                }
                            }
                            next = session.next_view() => match next {
                                Ok(view) => update(&worker_control, "connected", Some(view), None),
                                Err(error) => {
                                    update(&worker_control, "error", None, Some(format!("{error:#}")));
                                    break;
                                }
                            }
                        }
                    }
                    endpoint.close().await;
                    Ok(())
                })
            }));
            match outcome {
                Ok(Ok(())) => update(&worker_control, "disconnected", None, None),
                Ok(Err(error)) => update(&worker_control, "error", None, Some(format!("{error:#}"))),
                Err(_) => update(&worker_control, "error", None, Some("native session panicked".to_owned())),
            }
        });
    if let Err(error) = spawned {
        update(
            &control,
            "error",
            None,
            Some(format!("could not start session thread: {error}")),
        );
    }
    id
}

async fn wait_for_cancel(mut cancel: watch::Receiver<bool>) {
    if !*cancel.borrow() {
        let _ = cancel.changed().await;
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_developingtoday_id_NativeBridge_nativeStart<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    address: JString<'local>,
    world: JString<'local>,
    capability: JString<'local>,
) -> jlong {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(address) = java_string(&mut env, &address) else {
            return 0;
        };
        let Some(world) = java_string(&mut env, &world) else {
            return 0;
        };
        let Some(capability) = java_string(&mut env, &capability) else {
            return 0;
        };
        spawn_session(address, world, capability)
    }))
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_developingtoday_id_NativeBridge_nativeSnapshot<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    id: jlong,
) -> jstring {
    catch_unwind(AssertUnwindSafe(|| {
        let snapshot = SESSIONS
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(&id).cloned())
            .and_then(|control| {
                control
                    .snapshot
                    .lock()
                    .ok()
                    .map(|snapshot| snapshot.clone())
            })
            .unwrap_or(SessionSnapshot {
                status: "closed".to_owned(),
                view: None,
                error: None,
            });
        let json = serde_json::to_string(&snapshot).unwrap_or_else(|_| {
            "{\"status\":\"error\",\"view\":null,\"error\":\"snapshot serialization failed\"}"
                .to_owned()
        });
        env.new_string(json)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_developingtoday_id_NativeBridge_nativeStop(
    _env: JNIEnv<'_>,
    _this: JObject<'_>,
    id: jlong,
) {
    if let Ok(mut sessions) = SESSIONS.lock() {
        if let Some(session) = sessions.remove(&id) {
            let _ = session.cancel.send(true);
        }
    }
}
