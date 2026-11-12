//! Web security middleware.
//!
//! The web UI used to bind `0.0.0.0` with no authentication, so anyone who
//! could reach the port could overwrite or delete files. Three layered
//! defences now apply to every request:
//!
//! 1. **Bind address** (done in `serve`): loopback by default.
//! 2. **Host allow-list** ([`WebSecurity::allowed_hosts`]): when bound to
//!    loopback, requests whose `Host` header is not `localhost`/`127.0.0.1`/
//!    `[::1]` (or explicitly allowed) are refused, which defeats DNS-rebinding
//!    attacks from web pages in the user's browser.
//! 3. **Origin check**: a request carrying an `Origin` header (every browser
//!    cross-origin fetch and every WebSocket handshake) must come from the same
//!    origin (host *and* port) it is addressed to, which defeats cross-site
//!    request forgery and cross-site WebSocket hijacking. An origin with no
//!    explicit port only matches a `Host` with no explicit port — there is no
//!    scheme-default-port expansion, so `http://localhost` does not match a
//!    request addressed to `localhost:3000`.
//!
//! In addition, an optional **token** ([`WebSecurity::token`]) gates the whole
//! interface. It is accepted as the `id_token` cookie, an
//! `Authorization: Bearer` header, or a `?token=` query parameter; the latter
//! also sets the cookie so a browser only needs the link once,
//! and the bootstrap then redirects to the same path without the token
//! (#5746) so the token does not stay in the browser URL.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq as _;

/// Name of the cookie that carries the web token.
pub const TOKEN_COOKIE: &str = "id_token";

/// Security settings applied to every web request.
#[derive(Debug, Clone, Default)]
pub struct WebSecurity {
    /// If set, every request must present this token.
    pub token: Option<String>,
    /// Lower-case host names (no port) that may appear in the `Host` header.
    /// Empty disables the host check.
    pub allowed_hosts: Vec<String>,
    /// Mark the `id_token` cookie `Secure` (HTTPS in front).
    pub cookie_secure: bool,
}

/// `; Secure` when a session cookie must only travel over HTTPS.
pub(super) const fn secure_attribute(secure: bool) -> &'static str {
    if secure { "; Secure" } else { "" }
}

impl WebSecurity {
    /// Defaults for a server bound to `bind`.
    ///
    /// Loopback binds get a host allow-list; other binds rely on the token
    /// (if any) since the set of legitimate host names is unknown.
    pub fn for_bind(bind: std::net::IpAddr, token: Option<String>, extra_hosts: &[String]) -> Self {
        let mut allowed_hosts = Vec::new();
        if bind.is_loopback() {
            allowed_hosts.extend(["localhost", "127.0.0.1", "::1"].map(str::to_owned));
            allowed_hosts.extend(extra_hosts.iter().map(|h| h.to_lowercase()));
        }
        Self {
            token,
            allowed_hosts,
            cookie_secure: false,
        }
    }
}

/// Apply the security guard to a completed router tree.
///
/// Public router constructors call this once before returning, so embedders
/// cannot accidentally receive the unguarded route tree.
pub(super) fn protect(router: axum::Router, security: WebSecurity) -> axum::Router {
    router.layer(axum::middleware::from_fn_with_state(
        Arc::new(security),
        guard,
    ))
}

/// Strip an optional `:port` from a `Host`/`Origin` authority and lower-case it.
fn host_only(authority: &str) -> String {
    let a = authority.trim().to_lowercase();
    if let Some(rest) = a.strip_prefix('[') {
        // IPv6 literal: [::1]:3000 -- validate the WHOLE bracketed authority
        // before extracting the host (#8776). Only an empty tail or a
        // colon + numeric port may follow the closing bracket; anything
        // else (trailing host text, a non-numeric or empty port, a missing
        // bracket) is malformed and must not reduce to the bracketed host,
        // or the loopback allow-list would trust a mere prefix. Malformed
        // authorities return the full text, which matches no allowed host.
        let Some((host, tail)) = rest.split_once(']') else {
            return a.clone();
        };
        let tail_ok = tail.is_empty()
            || tail
                .strip_prefix(':')
                .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()));
        return if tail_ok { host.to_owned() } else { a.clone() };
    }
    a.rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or_else(|| a.clone(), |(h, _)| h.to_owned())
}

/// Normalise a `Host`/`Origin` authority for the Origin check: lower-case
/// the host and keep an explicit port, so authorities compare equal only
/// when both host and port match (#814). No port is invented: an authority
/// without a port stays port-less and therefore only matches another
/// port-less authority. IPv6 literals are re-bracketed (`[::1]:3000`).
fn authority_normalized(authority: &str) -> String {
    let a = authority.trim().to_lowercase();
    if let Some(rest) = a.strip_prefix('[') {
        // IPv6 literal: [::1]:3000 or [::1] -- validate the WHOLE bracketed
        // authority before normalizing (#8880), exactly as host_only does
        // for the Host side (#8776). Only an empty tail or a colon +
        // numeric port may follow the closing bracket; anything else
        // (trailing host text, a non-numeric or empty port, a missing
        // bracket) is malformed and must not reduce to the bracketed
        // authority, or a malformed Origin such as
        // http://[::1]attacker.example would normalize to [::1] and pass
        // the guard against Host [::1]. Malformed authorities return the
        // full text, which matches no valid normalized authority.
        let Some((host, tail)) = rest.split_once(']') else {
            return a.clone();
        };
        if tail.is_empty() {
            return format!("[{host}]");
        }
        return match tail
            .strip_prefix(':')
            .filter(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        {
            Some(port) => format!("[{host}]:{port}"),
            None => a.clone(),
        };
    }
    match a
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
    {
        Some((host, port)) => format!("{host}:{port}"),
        None => a,
    }
}

/// Authority (`host[:port]`) of an `Origin` header value such as `http://a:1`.
fn origin_authority(origin: &str) -> Option<&str> {
    origin
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
}

/// Constant-time byte comparison (length leaks, content does not).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then_some(v)
    })
}

/// The post-bootstrap URL for a request: its own path with the `token`
/// query parameter removed and every other parameter preserved verbatim
/// (#5746). Built only from the request URI, so the result is always a
/// same-origin relative reference and never an open redirect.
fn clean_bootstrap_target(uri: &axum::http::Uri) -> String {
    let path = uri.path();
    let kept: Vec<&str> = uri
        .query()
        .map(|q| {
            q.split('&')
                .filter(|kv| kv.split('=').next() != Some("token"))
                .collect()
        })
        .unwrap_or_default();
    if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

fn forbidden(msg: &'static str, status: StatusCode) -> Response {
    (status, msg).into_response()
}

/// Axum middleware enforcing [`WebSecurity`].
pub async fn guard(
    State(sec): State<Arc<WebSecurity>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let headers = req.headers();
    let host_header = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();

    // 2. Host allow-list.
    if !sec.allowed_hosts.is_empty() {
        let host = host_only(&host_header);
        if !sec.allowed_hosts.contains(&host) {
            return forbidden(
                "host not allowed (the web UI is bound to loopback; use --bind to expose it)",
                StatusCode::MISDIRECTED_REQUEST,
            );
        }
    }

    // 3. Origin must match the authority (host and port) the request is
    // addressed to (#814: a different port is a different origin).
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok())
        && origin != "null"
    {
        let same = origin_authority(origin)
            .is_some_and(|a| authority_normalized(a) == authority_normalized(&host_header));
        if !same {
            return forbidden("cross-origin request refused", StatusCode::FORBIDDEN);
        }
    } else if headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) == Some("null") {
        return forbidden("opaque origin refused", StatusCode::FORBIDDEN);
    }

    // Optional token.
    let mut set_cookie = None;
    if let Some(expected) = sec.token.as_ref() {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let cookie = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|c| cookie_value(c, TOKEN_COOKIE));
        let query = req.uri().query().and_then(|q| query_param(q, "token"));

        let ok_bearer = bearer.is_some_and(|t| ct_eq(t.as_bytes(), expected.as_bytes()));
        let ok_cookie = cookie.is_some_and(|t| ct_eq(t.as_bytes(), expected.as_bytes()));
        let ok_query = query.is_some_and(|t| ct_eq(t.as_bytes(), expected.as_bytes()));

        if !(ok_bearer || ok_cookie || ok_query) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "token required: open the link printed by `id serve` (it ends in ?token=...)",
            )
                .into_response();
        }
        if ok_query && !ok_cookie {
            set_cookie = HeaderValue::from_str(&format!(
                "{TOKEN_COOKIE}={expected}; HttpOnly; SameSite=Strict; Path=/{}",
                secure_attribute(sec.cookie_secure)
            ))
            .ok();
        }
    }

    // #5746: a successful query-token bootstrap must not leave the token
    // in the browser URL (history, copied links, Referer, access logs).
    // When this request just minted the cookie, answer the navigation
    // with a redirect to the clean target instead of serving the page
    // under the token URL. Non-GET requests keep the old behaviour: a
    // 303 would rewrite them to GET and drop the body.
    if set_cookie.is_some() && *req.method() == axum::http::Method::GET {
        if let Ok(loc) = HeaderValue::from_str(&clean_bootstrap_target(req.uri())) {
            let mut resp = StatusCode::SEE_OTHER.into_response();
            resp.headers_mut().insert(header::LOCATION, loc);
            if let Some(c) = set_cookie.take() {
                resp.headers_mut().append(header::SET_COOKIE, c);
            }
            return resp;
        }
    }

    let mut resp = next.run(req).await;
    if let Some(c) = set_cookie {
        resp.headers_mut().append(header::SET_COOKIE, c);
    }
    resp
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::{Router, middleware, routing::get};
    use tower::ServiceExt;

    fn app(sec: WebSecurity) -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(Arc::new(sec), guard))
    }

    async fn call(app: Router, req: Request<Body>) -> Response {
        app.oneshot(req).await.unwrap()
    }

    fn get_req(host: &str) -> axum::http::request::Builder {
        Request::builder().uri("/").header(header::HOST, host)
    }

    fn loopback() -> WebSecurity {
        WebSecurity::for_bind("127.0.0.1".parse().unwrap(), None, &[])
    }

    #[test]
    fn host_only_strips_ports_and_brackets() {
        assert_eq!(host_only("Localhost:3000"), "localhost");
        assert_eq!(host_only("127.0.0.1"), "127.0.0.1");
        assert_eq!(host_only("[::1]:3000"), "::1");
        assert_eq!(host_only("example.com"), "example.com");
    }

    #[test]
    fn host_only_rejects_malformed_bracketed_authorities() {
        // Oracle: a bracketed authority is well-formed only as a bracketed
        // host, optionally followed by a colon and a numeric port. Anything
        // else must NOT reduce to the bracketed host, or the loopback
        // allow-list would trust a mere prefix.
        for malformed in [
            "[::1]attacker.example",
            "[::1]attacker.example:3000",
            "[::1]:abc",
            "[::1]:",
            "[::1",
            "[::1]]",
        ] {
            assert_ne!(host_only(malformed), "::1", "{malformed}");
        }
        // Valid controls still extract the host.
        assert_eq!(host_only("[::1]"), "::1");
        assert_eq!(host_only("[::1]:3000"), "::1");
    }

    #[test]
    fn authority_normalized_rejects_malformed_bracketed_authorities() {
        // Oracle (#8880): a bracketed authority normalizes to the bracketed
        // form ONLY when it is well-formed -- bracketed host, optionally
        // followed by a colon and a numeric port. A malformed authority
        // must NOT reduce to the bracketed authority, or the Origin guard
        // would trust a mere prefix (Origin http://[::1]attacker.example
        // vs Host [::1]).
        for malformed in [
            "[::1]attacker.example",
            "[::1]attacker.example:3000",
            "[::1]:abc",
            "[::1]:",
            "[::1",
            "[::1]]",
        ] {
            assert_ne!(authority_normalized(malformed), "[::1]", "{malformed}");
            assert_ne!(authority_normalized(malformed), "[::1]:3000", "{malformed}");
        }
        // Valid controls still normalize, port included (#814).
        assert_eq!(authority_normalized("[::1]"), "[::1]");
        assert_eq!(authority_normalized("[::1]:3000"), "[::1]:3000");
        assert_eq!(authority_normalized("[::1]:9999"), "[::1]:9999");
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn bracketed_authority_collapses_only_for_valid_tails(
            host in proptest::sample::select(vec!["::1", "::", "fe80::1", "2001:db8::1"]),
            tail in "[!-~]{0,12}",
        ) {
            // Oracle (#8776): host_only of a bracketed authority returns the
            // bracketed host ONLY when the tail after the closing bracket is
            // empty or a colon followed by a non-empty all-digit port. Every
            // other tail leaves the authority unreduced, so it can never
            // match an allowed host. The fixed examples in
            // host_only_rejects_malformed_bracketed_authorities are the
            // regression smoke tests for this generated invariant.
            let authority = format!("[{host}]{tail}");
            let valid_tail = tail.is_empty()
                || tail
                    .strip_prefix(':')
                    .is_some_and(|port| {
                        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
                    });
            if valid_tail {
                prop_assert_eq!(host_only(&authority), host.to_owned(), "{}", authority);
            } else {
                prop_assert_ne!(host_only(&authority), host.to_owned(), "{}", authority);
            }
        }

        #[test]
        fn bracketed_authority_normalizes_only_for_valid_tails(
            host in proptest::sample::select(vec!["::1", "::", "fe80::1", "2001:db8::1"]),
            tail in "[!-~]{0,12}",
        ) {
            // Oracle (#8880): authority_normalized of a bracketed authority
            // returns the bracketed normalized form ONLY when the tail
            // after the closing bracket is empty or a colon followed by a
            // non-empty all-digit port. Every other tail leaves the
            // authority unreduced, so it can never equal a valid Host
            // authority. The fixed examples in
            // authority_normalized_rejects_malformed_bracketed_authorities
            // are the regression smoke tests for this invariant.
            let authority = format!("[{host}]{tail}");
            let valid_tail = tail.is_empty()
                || tail
                    .strip_prefix(':')
                    .is_some_and(|port| {
                        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
                    });
            if valid_tail {
                prop_assert_eq!(
                    authority_normalized(&authority),
                    authority.to_lowercase(),
                    "{}",
                    authority
                );
            } else {
                prop_assert_ne!(
                    authority_normalized(&authority),
                    format!("[{host}]"),
                    "{}",
                    authority
                );
            }
        }

        #[test]
        fn bracketed_authority_with_numeric_port_always_extracts_host(
            host in proptest::sample::select(vec!["::1", "::", "fe80::1", "2001:db8::1"]),
            port in any::<u16>(),
        ) {
            prop_assert_eq!(host_only(&format!("[{host}]:{port}")), host.to_owned());
            prop_assert_eq!(host_only(&format!("[{host}]")), host.to_owned());
        }
    }

    #[test]
    fn ct_eq_behaves_like_eq() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[tokio::test]
    async fn loopback_hosts_are_allowed() {
        for host in ["localhost:3000", "127.0.0.1:3000", "[::1]:3000"] {
            let r = call(app(loopback()), get_req(host).body(Body::empty()).unwrap()).await;
            assert_eq!(r.status(), StatusCode::OK, "{host}");
        }
    }

    #[tokio::test]
    async fn rebinding_host_is_refused() {
        let r = call(
            app(loopback()),
            get_req("evil.example:3000").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::MISDIRECTED_REQUEST);
    }

    #[tokio::test]
    async fn malformed_bracketed_host_is_refused() {
        for host in ["[::1]attacker.example", "[::1", "[::1]:abc"] {
            let r = call(app(loopback()), get_req(host).body(Body::empty()).unwrap()).await;
            assert_eq!(r.status(), StatusCode::MISDIRECTED_REQUEST, "{host}");
        }
        // Valid bracketed controls still pass the guard.
        for host in ["[::1]", "[::1]:3000"] {
            let r = call(app(loopback()), get_req(host).body(Body::empty()).unwrap()).await;
            assert_eq!(r.status(), StatusCode::OK, "{host}");
        }
    }

    #[tokio::test]
    async fn extra_hosts_extend_the_loopback_allow_list() {
        let sec = WebSecurity::for_bind(
            "127.0.0.1".parse().unwrap(),
            None,
            &["Dev.Local".to_owned()],
        );
        let r = call(
            app(sec),
            get_req("dev.local:3000").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn public_bind_skips_host_check() {
        let sec = WebSecurity::for_bind("0.0.0.0".parse().unwrap(), None, &[]);
        assert!(sec.allowed_hosts.is_empty());
        let r = call(
            app(sec),
            get_req("anything.example").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn same_origin_is_allowed_cross_origin_is_not() {
        let ok = call(
            app(loopback()),
            get_req("localhost:3000")
                .header(header::ORIGIN, "http://localhost:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);

        let bad = call(
            app(loopback()),
            get_req("localhost:3000")
                .header(header::ORIGIN, "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::FORBIDDEN);

        let null = call(
            app(loopback()),
            get_req("localhost:3000")
                .header(header::ORIGIN, "null")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(null.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn different_port_origin_is_refused() {
        // #814: same host, different port is a different origin.
        let r = call(
            app(loopback()),
            get_req("localhost:3000")
                .header(header::ORIGIN, "http://localhost:9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn portless_origin_does_not_match_explicit_port_host() {
        // #814 decision: no scheme-default-port expansion — a port-less
        // Origin only matches a port-less Host.
        let r = call(
            app(loopback()),
            get_req("localhost:3000")
                .header(header::ORIGIN, "http://localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        let ok = call(
            app(loopback()),
            get_req("localhost")
                .header(header::ORIGIN, "http://localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ipv6_origin_port_is_compared() {
        let ok = call(
            app(loopback()),
            get_req("[::1]:3000")
                .header(header::ORIGIN, "http://[::1]:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);

        let bad = call(
            app(loopback()),
            get_req("[::1]:3000")
                .header(header::ORIGIN, "http://[::1]:9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn malformed_bracketed_origin_is_refused() {
        // #8880 regression: a malformed bracketed Origin authority must
        // not normalize to the bracketed host and pass against Host [::1].
        for origin in [
            "http://[::1]attacker.example",
            "http://[::1]attacker.example:3000",
            "http://[::1]:abc",
            "http://[::1",
        ] {
            let r = call(
                app(loopback()),
                get_req("[::1]")
                    .header(header::ORIGIN, origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{origin}");
        }
        // Valid bracketed controls still pass the Origin guard (#814).
        for (host, origin) in [
            ("[::1]", "http://[::1]"),
            ("[::1]:3000", "http://[::1]:3000"),
        ] {
            let r = call(
                app(loopback()),
                get_req(host)
                    .header(header::ORIGIN, origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(r.status(), StatusCode::OK, "{origin} vs {host}");
        }
    }

    fn with_token() -> WebSecurity {
        WebSecurity {
            token: Some("s3cret".to_owned()),
            allowed_hosts: Vec::new(),
            cookie_secure: false,
        }
    }

    #[tokio::test]
    async fn the_token_cookie_is_secure_only_when_asked() {
        for secure in [false, true] {
            let sec = WebSecurity {
                cookie_secure: secure,
                ..with_token()
            };
            let query = call(
                app(sec),
                Request::builder()
                    .uri("/?token=s3cret")
                    .header(header::HOST, "h")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            let set = query
                .headers()
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap();
            let attributes: Vec<&str> = set.split(';').map(str::trim).collect();
            assert_eq!(attributes.contains(&"Secure"), secure, "{set}");
            assert!(attributes.contains(&"HttpOnly"), "{set}");
        }
    }

    #[tokio::test]
    async fn token_is_required_when_configured() {
        let r = call(app(with_token()), get_req("h").body(Body::empty()).unwrap()).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bearer_cookie_and_query_tokens_are_accepted() {
        let bearer = call(
            app(with_token()),
            get_req("h")
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(bearer.status(), StatusCode::OK);

        let cookie = call(
            app(with_token()),
            get_req("h")
                .header(header::COOKIE, "a=b; id_token=s3cret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(cookie.status(), StatusCode::OK);

        let query = call(
            app(with_token()),
            Request::builder()
                .uri("/?token=s3cret")
                .header(header::HOST, "h")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(query.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            query
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "/"
        );
        let set = query
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set.starts_with("id_token=s3cret;"));
        assert!(set.contains("HttpOnly"));
        assert!(set.contains("SameSite=Strict"));
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let r = call(
            app(with_token()),
            get_req("h")
                .header(header::AUTHORIZATION, "Bearer nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn query_bootstrap_redirect_preserves_other_params_and_hides_token() {
        let r = call(
            app(with_token()),
            Request::builder()
                .uri("/page?a=1&token=s3cret&b=2")
                .header(header::HOST, "h")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        let loc = r.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        assert_eq!(loc, "/page?a=1&b=2");
        assert!(!loc.contains("s3cret"));
        assert!(!loc.contains("token="));
        let set = r
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set.starts_with("id_token=s3cret"), "{set}");
        assert!(set.contains("HttpOnly"));
        assert!(set.contains("SameSite=Strict"));
    }

    #[tokio::test]
    async fn cookie_auth_with_query_token_does_not_redirect() {
        let r = call(
            app(with_token()),
            Request::builder()
                .uri("/?token=s3cret")
                .header(header::HOST, "h")
                .header(header::COOKIE, "id_token=s3cret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers().get(header::LOCATION).is_none());
    }

    #[tokio::test]
    async fn invalid_query_token_is_rejected_without_cookie_or_redirect() {
        let r = call(
            app(with_token()),
            Request::builder()
                .uri("/?token=wrong")
                .header(header::HOST, "h")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert!(r.headers().get(header::SET_COOKIE).is_none());
        assert!(r.headers().get(header::LOCATION).is_none());
    }
}
