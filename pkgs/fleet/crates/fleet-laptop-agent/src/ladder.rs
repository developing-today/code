//! The transport ladder: an ordered list of legs, tried in turn.
//!
//! The fall-through rule is what keeps the ladder safe to automate:
//!
//! * an absent leg (not configured here) or a leg that failed before
//!   the request left this machine is skipped, and the next leg is
//!   tried;
//! * a leg that failed after the request was written is retried on
//!   the next leg only when the request is idempotent. A non-idempotent
//!   request (an exec, a job submit) that may already have run on the
//!   laptop is never replayed: the call errors, naming the leg.

use std::fmt;

#[derive(Clone, Debug)]
pub struct Request {
    pub method: &'static str,
    pub path: String,
    pub body: Option<serde_json::Value>,
    pub trip: String,
    pub idempotent: bool,
    /// Raw-byte replies (GET /file) cannot cross the broker leg: its
    /// protocol is JSON-only. Direct legs only, like the Python client.
    pub direct_only: bool,
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum LegError {
    /// The leg is not configured for this request; nothing was tried.
    Absent(String),
    /// The leg failed before the request left this machine.
    Unsent(String),
    /// The request was written, then the leg failed.
    Sent(String),
}

pub trait Leg: Send + Sync {
    fn name(&self) -> &'static str;
    fn call(&self, req: &Request) -> Result<Reply, LegError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub leg: &'static str,
    pub kind: &'static str,
    pub reason: String,
}

#[derive(Debug, PartialEq)]
pub struct Answer {
    pub leg: &'static str,
    pub reply: Reply,
    /// Legs tried before the one that answered, and why each failed.
    pub fallbacks: Vec<Failure>,
}

#[derive(Debug, PartialEq)]
pub enum CallError {
    /// Every leg failed before the request reached the daemon.
    AllLegsFailed(Vec<Failure>),
    /// A non-idempotent request failed mid-flight and was not replayed.
    NotReplayed {
        failure: Failure,
        earlier: Vec<Failure>,
    },
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::AllLegsFailed(failures) => {
                write!(f, "all transport legs failed: {}", render(failures))
            }
            CallError::NotReplayed { failure, earlier } => {
                let mut all = earlier.clone();
                all.push(failure.clone());
                write!(
                    f,
                    "{} failed after the request was sent and was not replayed (not idempotent); legs: {}",
                    failure.leg,
                    render(&all)
                )
            }
        }
    }
}

fn render(failures: &[Failure]) -> String {
    failures
        .iter()
        .map(|f| format!("{}: {}", f.leg, f.reason))
        .collect::<Vec<_>>()
        .join("; ")
}

pub struct Ladder {
    legs: Vec<Box<dyn Leg>>,
}

impl Ladder {
    pub fn new(legs: Vec<Box<dyn Leg>>) -> Ladder {
        Ladder { legs }
    }

    pub fn call(&self, req: &Request) -> Result<Answer, CallError> {
        let mut failures: Vec<Failure> = Vec::new();
        for leg in &self.legs {
            let name = leg.name();
            if req.direct_only && name == "broker" {
                continue;
            }
            match leg.call(req) {
                Ok(reply) => {
                    return Ok(Answer {
                        leg: name,
                        reply,
                        fallbacks: failures,
                    });
                }
                Err(LegError::Absent(reason)) => failures.push(Failure {
                    leg: name,
                    kind: "absent",
                    reason,
                }),
                Err(LegError::Unsent(reason)) => failures.push(Failure {
                    leg: name,
                    kind: "unsent",
                    reason,
                }),
                Err(LegError::Sent(reason)) => {
                    let failure = Failure {
                        leg: name,
                        kind: "sent",
                        reason,
                    };
                    if !req.idempotent {
                        return Err(CallError::NotReplayed {
                            failure,
                            earlier: failures,
                        });
                    }
                    failures.push(failure);
                }
            }
        }
        Err(CallError::AllLegsFailed(failures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct Fake {
        name: &'static str,
        outcome: Mutex<Option<Result<Reply, LegError>>>,
        calls: Arc<AtomicUsize>,
    }

    impl Leg for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn call(&self, _req: &Request) -> Result<Reply, LegError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcome.lock().unwrap().take().expect("one call per fake")
        }
    }

    fn fake(name: &'static str, outcome: Result<Reply, LegError>) -> (Box<dyn Leg>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let leg = Fake {
            name,
            outcome: Mutex::new(Some(outcome)),
            calls: calls.clone(),
        };
        (Box::new(leg), calls)
    }

    fn ok(status: u16) -> Result<Reply, LegError> {
        Ok(Reply {
            status,
            body: b"{}".to_vec(),
        })
    }

    fn req(idempotent: bool) -> Request {
        Request {
            method: "POST",
            path: "/exec".into(),
            body: None,
            trip: "t".into(),
            idempotent,
            direct_only: false,
        }
    }

    #[test]
    fn a_dead_leg_degrades_to_the_next_and_names_it() {
        let (a, a_calls) = fake("broker", Err(LegError::Absent("no socket".into())));
        let (b, b_calls) = fake("cloudflare", Err(LegError::Unsent("connect refused".into())));
        let (c, _) = fake("tailscale", ok(200));
        let ladder = Ladder::new(vec![a, b, c]);
        let answer = ladder.call(&req(false)).unwrap();
        assert_eq!(answer.leg, "tailscale");
        assert_eq!(answer.reply.status, 200);
        assert_eq!(answer.fallbacks.len(), 2);
        assert_eq!(answer.fallbacks[1].kind, "unsent");
        assert_eq!(answer.fallbacks[1].reason, "connect refused");
        assert_eq!(a_calls.load(Ordering::SeqCst), 1);
        assert_eq!(b_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn total_failure_names_every_leg() {
        let (a, _) = fake("broker", Err(LegError::Absent("no socket".into())));
        let (b, _) = fake("tailscale", Err(LegError::Unsent("HTTPS_PROXY unset".into())));
        let err = Ladder::new(vec![a, b]).call(&req(true)).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("broker: no socket"), "{text}");
        assert!(text.contains("tailscale: HTTPS_PROXY unset"), "{text}");
    }

    #[test]
    fn a_sent_non_idempotent_request_is_never_replayed() {
        let (a, _) = fake("tailscale", Err(LegError::Sent("reset by peer".into())));
        let (b, b_calls) = fake("cloudflare", ok(200));
        let err = Ladder::new(vec![a, b]).call(&req(false)).unwrap_err();
        assert!(matches!(err, CallError::NotReplayed { .. }), "{err:?}");
        assert_eq!(b_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_sent_idempotent_request_falls_through() {
        let (a, _) = fake("tailscale", Err(LegError::Sent("reset by peer".into())));
        let (b, _) = fake("cloudflare", ok(200));
        let answer = Ladder::new(vec![a, b]).call(&req(true)).unwrap();
        assert_eq!(answer.leg, "cloudflare");
        assert_eq!(answer.fallbacks[0].kind, "sent");
    }

    #[test]
    fn an_http_error_status_is_an_answer_not_a_fallback() {
        let (a, _) = fake("broker", ok(401));
        let (b, b_calls) = fake("tailscale", ok(200));
        let answer = Ladder::new(vec![a, b]).call(&req(true)).unwrap();
        assert_eq!(answer.reply.status, 401);
        assert_eq!(b_calls.load(Ordering::SeqCst), 0);
    }
}
