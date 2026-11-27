//! #14191 (fleet context propagation build 5/5): attribution at the
//! queue's event store.
//!
//! The queue broker serves each request on one thread, start to
//! finish. While it serves a request that carried `X-Fleet-Ctx-*`
//! headers, [`enter`] parks that request's context id in a
//! thread-local, and [`stamp`] adds it to the `detail` of every
//! `events` row the request writes as `ctx_id`. One context id then
//! joins the broker's trip log line (`FLEET_CTX_ID=…`, #14188) and
//! the durable queue events (`repo-queue events <task>`) for the same
//! originating action, with no model involvement and no added round
//! trip.
//!
//! Absence stays representable: a request without context, a
//! background sweeper, or any thread the broker spawns writes its
//! rows exactly as before. Only the id travels (the context is ids
//! and hashes only, never secrets), and a `detail` that already names
//! a `ctx_id`, or that is not a JSON object, is left as the caller
//! wrote it.

use std::cell::RefCell;

use serde_json::Value;

/// The `detail` key the context id is stamped under.
pub const CTX_ID_DETAIL_KEY: &str = "ctx_id";

thread_local! {
    static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Restores the previous context id when the request finishes.
#[must_use = "the context id is cleared again when the scope drops"]
pub struct CtxScope {
    previous: Option<String>,
}

impl Drop for CtxScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|current| *current.borrow_mut() = previous);
    }
}

/// Attribute events written on this thread to `id` until the returned
/// scope drops. `None` (a request with no context) clears any id.
pub fn enter(id: Option<&str>) -> CtxScope {
    let previous = CURRENT.with(|current| current.replace(id.map(str::to_string)));
    CtxScope { previous }
}

/// The context id of the request this thread is serving, if any.
pub fn current_id() -> Option<String> {
    CURRENT.with(|current| current.borrow().clone())
}

/// `detail` with the current context id added under `ctx_id`, or
/// `detail` unchanged when there is no current id, the detail is not
/// an object, or it already carries a `ctx_id`.
pub fn stamp(detail: &Value) -> Value {
    let Some(id) = current_id() else {
        return detail.clone();
    };
    match detail {
        Value::Object(map) if !map.contains_key(CTX_ID_DETAIL_KEY) => {
            let mut map = map.clone();
            map.insert(CTX_ID_DETAIL_KEY.to_string(), Value::String(id));
            Value::Object(map)
        }
        _ => detail.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stamps_only_inside_a_scope_and_restores_on_drop() {
        assert_eq!(stamp(&json!({"a": 1})), json!({"a": 1}));
        {
            let _scope = enter(Some("task-22655"));
            assert_eq!(
                stamp(&json!({"a": 1})),
                json!({"a": 1, "ctx_id": "task-22655"})
            );
            {
                let _inner = enter(None);
                assert_eq!(stamp(&json!({})), json!({}));
            }
            assert_eq!(current_id().as_deref(), Some("task-22655"));
        }
        assert_eq!(current_id(), None);
        assert_eq!(stamp(&json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn leaves_non_objects_and_existing_ids_alone() {
        let _scope = enter(Some("task-1"));
        assert_eq!(stamp(&json!("text")), json!("text"));
        assert_eq!(stamp(&json!(null)), json!(null));
        assert_eq!(
            stamp(&json!({"ctx_id": "caller"})),
            json!({"ctx_id": "caller"})
        );
    }

    #[test]
    fn other_threads_see_no_context() {
        let _scope = enter(Some("task-1"));
        let seen = std::thread::spawn(current_id).join().unwrap();
        assert_eq!(seen, None);
    }
}
