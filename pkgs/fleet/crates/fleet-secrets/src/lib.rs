//! fleet-secrets — the one secrets-by-name path for fleet
//! (design note §1, epic developing-today/code#5412).
//!
//! Platform rule, carried over from the brokers' lessons: **secrets are
//! resolved by name through this crate only.** No domain shells out to
//! `bws` or reads a token file on its own. Resolution order:
//!
//!   1. the **secrets-broker protocol** — a framed request through the
//!      session broker to port 4800, `GET /secret/<name>`, with the
//!      token *file* named in the request (the session broker reads it
//!      per request and sends `X-Agent-Token`; fleet never opens it);
//!   2. the **`bws` CLI fallback** — `bws secret list` + `get`, the
//!      same shape the secrets broker itself uses to fill its cache.
//!
//! Discipline, enforced by the types below:
//!   * a resolved value lives in [`SecretValue`], whose `Debug` is
//!     redacted and which exposes the string only through `expose()`;
//!   * errors name the secret's *name*, the source, and status codes —
//!     never a value, never a subprocess's stderr (which could quote
//!     vault material);
//!   * nothing here persists a value: no cache, no file, no log line.
//!   * an empty string is never a successful resolution (#6275): a
//!     broken read and a genuinely empty vault value are
//!     indistinguishable at this layer, and the 2026-11-05 incident
//!     was a caller exiting 0 with an empty value while its sources
//!     had actually failed — so an empty value counts as a source
//!     failure everywhere in this crate, and the chain moves on or
//!     fails loudly.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;

use fleet_proxy_proto::{Request, Response};

/// A resolved secret value. Construct only from a source; the inner
/// string is reachable only via [`SecretValue::expose`].
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap a value a [`SecretSource`] has just resolved from its
    /// backend. Source implementations outside this crate construct
    /// values only here, so every resolved secret immediately sits
    /// behind the redacted newtype.
    pub fn from_resolved(value: String) -> SecretValue {
        SecretValue(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(***)")
    }
}

/// Resolution failures. Deliberately value-free (see crate docs).
#[derive(Debug)]
pub enum SecretError {
    /// The name itself is not usable (it becomes a URL path segment,
    /// so separators and controls are rejected before any I/O).
    InvalidName(String),
    /// The source answered: no secret by that name.
    NotFound { source: &'static str, name: String },
    /// The source could not answer (transport down, bad status,
    /// subprocess failed). `detail` carries status codes and error
    /// kinds only — never payload or stderr content.
    Unavailable {
        source: &'static str,
        name: String,
        detail: String,
    },
    /// Every source in the chain failed; carries each source's error.
    Chain {
        name: String,
        errors: Vec<SecretError>,
    },
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::InvalidName(name) => {
                write!(
                    f,
                    "invalid secret name {name:?}: names may not contain '/', '?', '#', '%', whitespace, or control characters, and may not be '.' or '..'"
                )
            }
            SecretError::NotFound { source, name } => {
                write!(f, "secret {name:?} not found via {source}")
            }
            SecretError::Unavailable {
                source,
                name,
                detail,
            } => write!(f, "secret {name:?} unavailable via {source}: {detail}"),
            SecretError::Chain { name, errors } => {
                write!(
                    f,
                    "secret {name:?} unresolved: {} source(s) failed",
                    errors.len()
                )?;
                for e in errors {
                    write!(f, "; {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SecretError {}

/// Names become URL path segments (`GET /secret/<name>`); validate
/// before any source sees them.
pub fn validate_name(name: &str) -> Result<(), SecretError> {
    // A name is interpolated raw as one path segment, so the encoded
    // and dot-segment spellings of the refused characters are refused
    // too (issue #5469): a percent sign introduces every percent-encoded
    // spelling (an encoded slash decodes to a separator, an encoded
    // dot pair decodes to a dot segment), and a name that is exactly a
    // dot segment is removed by path normalization — the request would
    // no longer name a secret under the secret prefix at all.
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|c| {
            c == '/' || c == '?' || c == '#' || c == '%' || c.is_whitespace() || c.is_control()
        })
    {
        return Err(SecretError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// One way to resolve a name. Sources are tried in chain order.
pub trait SecretSource {
    /// Stable source name for error messages ("secrets-broker", "bws").
    fn source_name(&self) -> &'static str;
    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError>;
}

/// The ordered chain: broker protocol first, `bws` fallback (§1).
pub struct Resolver {
    sources: Vec<Box<dyn SecretSource>>,
}

impl Resolver {
    pub fn new(sources: Vec<Box<dyn SecretSource>>) -> Resolver {
        Resolver { sources }
    }

    /// The default chain for a resolved fleet config.
    ///
    /// A broker path-resolution failure (e.g. `HOME` unavailable, so
    /// `broker_socket()` / `secrets_token_file()` cannot resolve) is
    /// a broker-*source* failure, not a chain-construction failure
    /// (#6826): the chain is built with a broker source that reports
    /// the recorded path error, so resolution still attempts the
    /// `bws` fallback, and the broker error rides the chain's error
    /// list when every source fails. When the paths resolve, the
    /// chain is exactly what it was before. The `Result` stays for
    /// signature stability; path failures no longer produce `Err`.
    pub fn default_chain(
        config: &fleet_config::Config,
    ) -> Result<Resolver, fleet_config::ConfigError> {
        let paths = config.broker_socket().and_then(|broker_socket| {
            config
                .secrets_token_file()
                .map(|token_file| (broker_socket, token_file))
        });
        Ok(Resolver::new(vec![
            broker_source(paths),
            Box::new(BwsSource::new()),
        ]))
    }

    pub fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        let mut errors = Vec::new();
        for source in &self.sources {
            match source.resolve(name) {
                Ok(value) if !value.expose().is_empty() => return Ok(value),
                Ok(_) => errors.push(SecretError::Unavailable {
                    source: source.source_name(),
                    name: name.to_string(),
                    detail: "source returned an empty value".to_string(),
                }),
                Err(e) => errors.push(e),
            }
        }
        Err(SecretError::Chain {
            name: name.to_string(),
            errors,
        })
    }
}

/// The broker source for a pair of resolved broker paths, or —
/// when path resolution itself failed (#6826) — a source that can
/// only report that failure. Split from `Resolver::default_chain`
/// so the failure shape is testable without bending the process
/// environment.
fn broker_source(
    paths: Result<(PathBuf, PathBuf), fleet_config::ConfigError>,
) -> Box<dyn SecretSource> {
    match paths {
        Ok((broker_socket, token_file)) => Box::new(BrokerSource {
            broker_socket,
            token_file,
        }),
        Err(e) => Box::new(BrokerPathErrorSource {
            detail: e.to_string(),
        }),
    }
}

/// A secrets-broker source whose paths never resolved: every
/// resolution attempt fails with the recorded path error, so the
/// chain moves on to the next source and the error is retained in
/// the chain's failure list (#6826).
struct BrokerPathErrorSource {
    detail: String,
}

impl SecretSource for BrokerPathErrorSource {
    fn source_name(&self) -> &'static str {
        "secrets-broker"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        Err(SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail: self.detail.clone(),
        })
    }
}

/// Source 1: the secrets-broker protocol, through the session broker.
/// The exchange is one framed [`Request`] / [`Response`] pair over the
/// session broker's Unix socket — the same path every sandbox client
/// uses today.
pub struct BrokerSource {
    pub broker_socket: PathBuf,
    /// Path of the token file the session broker reads per request.
    /// fleet names it in the request and never opens it.
    pub token_file: PathBuf,
}

impl BrokerSource {
    /// One framed exchange over an already-connected session-broker
    /// stream. Split from `resolve` so tests can drive it over a
    /// socket pair.
    fn exchange(
        &self,
        stream: &mut UnixStream,
        request: &Request,
        name: &str,
    ) -> Result<Response, SecretError> {
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail,
        };
        fleet_proxy_proto::write_frame(stream, request).map_err(|e| unavailable(e.to_string()))?;
        let response: Response =
            fleet_proxy_proto::read_typed(stream).map_err(|e| unavailable(e.to_string()))?;
        Ok(response)
    }

    /// One framed request/response over a fresh session-broker
    /// connection — the exchange [`BrokerSource::resolve`] and
    /// [`BrokerSource::list_names`] share.
    fn request(&self, path: &str, name: &str) -> Result<Response, SecretError> {
        let request = Request {
            method: "GET".to_string(),
            path: path.to_string(),
            body: None,
            port: Some(fleet_config::PORT_SECRETS_BROKER),
            token_file: Some(self.token_file.to_string_lossy().into_owned()),
        };
        let mut stream = self.connect(name)?;
        self.exchange(&mut stream, &request, name)
    }

    /// The broker's names listing (`GET /secrets` — the endpoint the
    /// step-6 verb's `list` renders). Names only: the broker's reply
    /// carries no values on this route, and neither does this API.
    /// Errors are [`SecretError::Unavailable`] under the pseudo-name
    /// `"names"` — a listing has no single secret to name.
    pub fn list_names(&self) -> Result<Vec<String>, SecretError> {
        let name = "names";
        let response = self.request("/secrets", name)?;
        match response.status {
            200 => {
                let names = response
                    .body
                    .get("names")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response without a \"names\" array".to_string(),
                    })?;
                names
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| SecretError::Unavailable {
                                source: "secrets-broker",
                                name: name.to_string(),
                                detail: "\"names\" array holds a non-string entry".to_string(),
                            })
                    })
                    .collect()
            }
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: format!("secrets-broker answered HTTP {status}"),
            }),
        }
    }

    fn connect(&self, name: &str) -> Result<UnixStream, SecretError> {
        UnixStream::connect(&self.broker_socket).map_err(|e| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail: format!(
                "session broker socket {}: {}",
                self.broker_socket.display(),
                e.kind()
            ),
        })
    }
}

impl SecretSource for BrokerSource {
    fn source_name(&self) -> &'static str {
        "secrets-broker"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        let response = self.request(&format!("/secret/{name}"), name)?;
        match response.status {
            200 => {
                let value = response
                    .body
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response without a string \"value\" field".to_string(),
                    })?;
                if value.is_empty() {
                    return Err(SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response with an empty \"value\"".to_string(),
                    });
                }
                Ok(SecretValue(value.to_string()))
            }
            404 => Err(SecretError::NotFound {
                source: "secrets-broker",
                name: name.to_string(),
            }),
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: format!("secrets-broker answered HTTP {status}"),
            }),
        }
    }
}

/// Runs external commands for the `bws` fallback. Injectable so tests
/// never spawn a real `bws` (and never see a real value).
pub trait CommandRunner {
    /// Run `program args`, returning stdout on success. Error details
    /// carry the exit status only — stderr is deliberately not captured
    /// into errors (it could quote vault material).
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("spawn failed: {}", e.kind()))?;
        if !out.status.success() {
            return Err(format!("exit status {}", out.status));
        }
        String::from_utf8(out.stdout).map_err(|_| "output not utf8".to_string())
    }
}

/// Source 2: the `bws` CLI fallback — `bws secret list` to find the
/// id for a name, then `bws secret get <id>`, exactly the shape the
/// secrets broker's own cache refresh uses.
pub struct BwsSource {
    runner: Box<dyn CommandRunner>,
}

impl BwsSource {
    pub fn new() -> BwsSource {
        BwsSource {
            runner: Box::new(ProcessRunner),
        }
    }

    pub fn with_runner(runner: Box<dyn CommandRunner>) -> BwsSource {
        BwsSource { runner }
    }
}

impl Default for BwsSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretSource for BwsSource {
    fn source_name(&self) -> &'static str {
        "bws"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "bws",
            name: name.to_string(),
            detail,
        };
        let list_raw = self
            .runner
            .run("bws", &["secret", "list", "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret list: {d}")))?;
        let list: serde_json::Value = serde_json::from_str(&list_raw)
            .map_err(|_| unavailable("bws secret list: output is not JSON".to_string()))?;
        let id = list
            .as_array()
            .and_then(|items| {
                items.iter().find_map(|item| {
                    if item.get("key").and_then(|k| k.as_str()) == Some(name) {
                        item.get("id").and_then(|i| i.as_str()).map(str::to_string)
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| SecretError::NotFound {
                source: "bws",
                name: name.to_string(),
            })?;
        let got_raw = self
            .runner
            .run("bws", &["secret", "get", &id, "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret get: {d}")))?;
        let got: serde_json::Value = serde_json::from_str(&got_raw)
            .map_err(|_| unavailable("bws secret get: output is not JSON".to_string()))?;
        let value = got
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| unavailable("bws secret get: no string \"value\" field".to_string()))?;
        if value.is_empty() {
            return Err(unavailable(
                "bws secret get: empty \"value\" field".to_string(),
            ));
        }
        Ok(SecretValue(value.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSource {
        name: &'static str,
        result: Result<String, SecretError>,
    }

    impl SecretSource for FakeSource {
        fn source_name(&self) -> &'static str {
            self.name
        }
        fn resolve(&self, _name: &str) -> Result<SecretValue, SecretError> {
            match &self.result {
                Ok(v) => Ok(SecretValue(v.clone())),
                Err(_) => Err(SecretError::Unavailable {
                    source: self.name,
                    name: "x".to_string(),
                    detail: "down".to_string(),
                }),
            }
        }
    }

    #[test]
    fn chain_tries_sources_in_order() {
        let resolver = Resolver::new(vec![
            Box::new(FakeSource {
                name: "secrets-broker",
                result: Err(SecretError::Unavailable {
                    source: "secrets-broker",
                    name: "x".into(),
                    detail: "down".into(),
                }),
            }),
            Box::new(FakeSource {
                name: "bws",
                result: Ok("fixture-value-not-a-secret".to_string()),
            }),
        ]);
        let v = resolver.resolve("some.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn chain_skips_an_empty_value_and_uses_the_next_source() {
        let resolver = Resolver::new(vec![
            Box::new(FakeSource {
                name: "secrets-broker",
                result: Ok(String::new()),
            }),
            Box::new(FakeSource {
                name: "bws",
                result: Ok("fixture-value-not-a-secret".to_string()),
            }),
        ]);
        let v = resolver.resolve("some.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn chain_of_only_empty_values_fails_loudly() {
        let resolver = Resolver::new(vec![Box::new(FakeSource {
            name: "secrets-broker",
            result: Ok(String::new()),
        })]);
        let err = resolver.resolve("some.name").unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains("empty value"),
            "the failure names the empty read: {text}"
        );
    }

    #[test]
    fn chain_reports_all_failures_without_values() {
        let resolver = Resolver::new(vec![Box::new(FakeSource {
            name: "bws",
            result: Err(SecretError::Unavailable {
                source: "bws",
                name: "x".into(),
                detail: "down".into(),
            }),
        })]);
        let err = resolver.resolve("some.name").unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("some.name"), "error names the secret: {text}");
        assert!(
            !text.contains("fixture"),
            "error must not carry values: {text}"
        );
    }

    #[test]
    fn broker_path_error_is_a_broker_source_failure_not_a_chain_failure() {
        // #6826: with the broker paths unresolvable, the broker
        // source reports the recorded path error under the broker's
        // own name — it does not abort chain construction.
        let source = broker_source(Err(fleet_config::ConfigError::MissingHome {
            setting: "the default broker socket path",
        }));
        assert_eq!(source.source_name(), "secrets-broker");
        let err = source.resolve("some.name").unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains("HOME is unset or empty"),
            "the recorded path error is the broker failure: {text}"
        );
    }

    #[test]
    fn broker_path_error_still_falls_through_to_the_bws_fallback() {
        // #6826: a chain holding the path-error broker source
        // resolves through the next source in the chain.
        let resolver = Resolver::new(vec![
            broker_source(Err(fleet_config::ConfigError::MissingHome {
                setting: "the default broker socket path",
            })),
            Box::new(FakeSource {
                name: "bws",
                result: Ok("fixture-value-not-a-secret".to_string()),
            }),
        ]);
        let v = resolver.resolve("some.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn broker_path_error_is_retained_when_every_source_fails() {
        // #6826: when the fallback also fails, the chain's error
        // still carries the broker path error.
        let resolver = Resolver::new(vec![
            broker_source(Err(fleet_config::ConfigError::MissingHome {
                setting: "the default broker socket path",
            })),
            Box::new(FakeSource {
                name: "bws",
                result: Err(SecretError::Unavailable {
                    source: "bws",
                    name: "x".into(),
                    detail: "down".into(),
                }),
            }),
        ]);
        let err = resolver.resolve("some.name").unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains("HOME is unset or empty"),
            "chain error retains the broker path error: {text}"
        );
        assert!(
            text.contains("bws"),
            "chain error names the fallback too: {text}"
        );
    }

    #[test]
    fn default_chain_builds_broker_then_bws_when_paths_resolve() {
        // Unchanged behaviour on the happy path (#6826): the real
        // broker source first, the bws fallback second.
        let config = fleet_config::Config::default();
        let resolver = Resolver::default_chain(&config).unwrap();
        let names: Vec<&str> = resolver
            .sources
            .iter()
            .map(|s| s.source_name())
            .collect();
        assert_eq!(names, ["secrets-broker", "bws"]);
    }

    #[test]
    fn debug_is_redacted() {
        let v = SecretValue("fixture-value-not-a-secret".to_string());
        let text = format!("{v:?}");
        assert_eq!(text, "SecretValue(***)");
    }

    #[test]
    fn name_validation_rejects_path_material() {
        for bad in [
            "", "a/b", "a?b", "a#b", "a b", "a\tb", "a\u{7}b", ".", "..", "%2e%2e", "%2f", "a%2fb",
            "a%3fb", "a%23b",
        ] {
            assert!(validate_name(bad).is_err(), "name {bad:?} must be rejected");
        }
        for good in [
            "supabase.service-role-key",
            "openclaw.gateway-token",
            "a.b-c_d",
        ] {
            assert!(validate_name(good).is_ok(), "name {good:?} must pass");
        }
    }

    struct FakeRunner {
        list: String,
        get: String,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            assert_eq!(program, "bws");
            match args {
                ["secret", "list", "-o", "json"] => Ok(self.list.clone()),
                ["secret", "get", _, "-o", "json"] => Ok(self.get.clone()),
                other => Err(format!("unexpected args: {other:?}")),
            }
        }
    }

    #[test]
    fn bws_fallback_lists_then_gets_by_name() {
        let source = BwsSource::with_runner(Box::new(FakeRunner {
            list: r#"[{"id":"id-1","key":"wanted.name"},{"id":"id-2","key":"other.name"}]"#
                .to_string(),
            get: r#"{"id":"id-1","key":"wanted.name","value":"fixture-value-not-a-secret"}"#
                .to_string(),
        }));
        let v = source.resolve("wanted.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
        match source.resolve("absent.name") {
            Err(SecretError::NotFound { source, .. }) => assert_eq!(source, "bws"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn bws_fallback_rejects_an_empty_value() {
        let source = BwsSource::with_runner(Box::new(FakeRunner {
            list: r#"[{"id":"id-1","key":"wanted.name"}]"#.to_string(),
            get: r#"{"id":"id-1","key":"wanted.name","value":""}"#.to_string(),
        }));
        match source.resolve("wanted.name") {
            Err(SecretError::Unavailable { source, detail, .. }) => {
                assert_eq!(source, "bws");
                assert!(detail.contains("empty"), "detail: {detail}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn broker_list_names_roundtrip_over_a_temp_socket() {
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("fleet-secrets-list-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("broker.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let req: Request = fleet_proxy_proto::read_typed(&mut stream).unwrap();
            assert_eq!(req.method, "GET");
            assert_eq!(req.path, "/secrets");
            assert_eq!(req.port, Some(fleet_config::PORT_SECRETS_BROKER));
            let resp = Response {
                status: 200,
                body: serde_json::json!({"names": ["a.name", "b.name"]}),
            };
            fleet_proxy_proto::write_frame(&mut stream, &resp).unwrap();
        });
        let source = BrokerSource {
            broker_socket: sock,
            token_file: PathBuf::from("/nonexistent/token"),
        };
        let names = source.list_names().unwrap();
        assert_eq!(names, vec!["a.name".to_string(), "b.name".to_string()]);
        handle.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broker_resolve_rejects_an_empty_value_over_a_temp_socket() {
        use std::os::unix::net::UnixListener;

        fn scripted_resolve(body: serde_json::Value) -> Result<SecretValue, SecretError> {
            let dir = std::env::temp_dir().join(format!(
                "fleet-secrets-resolve-test-{}-{}",
                std::process::id(),
                body["value"].as_str().map_or(0, str::len)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let sock = dir.join("broker.sock");
            let listener = UnixListener::bind(&sock).unwrap();
            let handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let req: Request = fleet_proxy_proto::read_typed(&mut stream).unwrap();
                assert_eq!(req.path, "/secret/wanted.name");
                let resp = Response { status: 200, body };
                fleet_proxy_proto::write_frame(&mut stream, &resp).unwrap();
            });
            let source = BrokerSource {
                broker_socket: sock,
                token_file: PathBuf::from("/nonexistent/token"),
            };
            let out = source.resolve("wanted.name");
            handle.join().unwrap();
            std::fs::remove_dir_all(&dir).ok();
            out
        }

        let ok = scripted_resolve(serde_json::json!({
            "key": "wanted.name",
            "value": "fixture-value-not-a-secret"
        }))
        .unwrap();
        assert_eq!(ok.expose(), "fixture-value-not-a-secret");

        match scripted_resolve(serde_json::json!({"key": "wanted.name", "value": ""})) {
            Err(SecretError::Unavailable { source, detail, .. }) => {
                assert_eq!(source, "secrets-broker");
                assert!(detail.contains("empty"), "detail: {detail}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn broker_source_roundtrip_over_a_socket_pair() {
        // A real Unix socket pair with a scripted broker end: proves
        // BrokerSource's framed exchange + status mapping (200, 404)
        // without any live service or real secret.
        use std::os::unix::net::UnixStream as Pair;
        use std::path::PathBuf;

        fn scripted_broker(
            status: u16,
            body: serde_json::Value,
        ) -> (BrokerSource, std::thread::JoinHandle<()>, Pair) {
            let (client, mut server) = Pair::pair().unwrap();
            let handle = std::thread::spawn(move || {
                let req: Request = fleet_proxy_proto::read_typed(&mut server).unwrap();
                assert_eq!(req.method, "GET");
                assert_eq!(req.path, "/secret/wanted.name");
                assert_eq!(req.port, Some(fleet_config::PORT_SECRETS_BROKER));
                assert_eq!(req.token_file.as_deref(), Some("/nonexistent/token"));
                let resp = Response { status, body };
                fleet_proxy_proto::write_frame(&mut server, &resp).unwrap();
            });
            let source = BrokerSource {
                broker_socket: PathBuf::from("/nonexistent/broker.sock"),
                token_file: PathBuf::from("/nonexistent/token"),
            };
            (source, handle, client)
        }

        // 200 -> value
        let (source, handle, mut client) = scripted_broker(
            200,
            serde_json::json!({"key": "wanted.name", "value": "fixture-value-not-a-secret"}),
        );
        let request = Request {
            method: "GET".to_string(),
            path: "/secret/wanted.name".to_string(),
            body: None,
            port: Some(fleet_config::PORT_SECRETS_BROKER),
            token_file: Some("/nonexistent/token".to_string()),
        };
        let resp = source
            .exchange(&mut client, &request, "wanted.name")
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["value"], "fixture-value-not-a-secret");
        handle.join().unwrap();

        // The resolve() status mapping, exercised against exchange()
        // results by shape: 404 maps to NotFound in resolve; here we
        // assert the raw status the mapping consumes.
        let (source, handle, mut client) =
            scripted_broker(404, serde_json::json!({"error": "not found"}));
        let resp = source
            .exchange(&mut client, &request, "wanted.name")
            .unwrap();
        assert_eq!(resp.status, 404);
        handle.join().unwrap();
    }
}
