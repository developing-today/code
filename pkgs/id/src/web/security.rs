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
//!    host it is addressed to, which defeats cross-site request forgery and
//!    cross-site WebSocket hijacking.
//!
//! In addition, an optional **token** ([`WebSecurity::token`]) gates the whole
//! interface. It is accepted as the `id_token` cookie, an
//! `Authorization: Bearer` header, or a `?token=` query parameter; the latter
//! also sets the cookie so a browser only needs the link once.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderValue, Method, Request, StatusCode, header},
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

/// Strip an optional `:port` from a `Host`/`Origin` authority and lower-case it.
fn host_only(authority: &str) -> String {
    let a = authority.trim().to_lowercase();
    if let Some(rest) = a.strip_prefix('[') {
        // IPv6 literal: [::1]:3000
        return rest.split(']').next().unwrap_or("").to_owned();
    }
    a.rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or_else(|| a.clone(), |(h, _)| h.to_owned())
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

    // 3. Origin must match the host the request is addressed to.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok())
        && origin != "null"
    {
        let same =
            origin_authority(origin).is_some_and(|a| host_only(a) == host_only(&host_header));
        if !same {
            return forbidden("cross-origin request refused", StatusCode::FORBIDDEN);
        }
    } else if headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) == Some("null") {
        return forbidden("opaque origin refused", StatusCode::FORBIDDEN);
    }

    // Optional token. Peer servers post signed envelopes without a session.
    let mut set_cookie = None;
    let peer_envelope =
        req.method() == Method::POST && req.uri().path() == super::explore::ENVELOPE_PATH;
    if let Some(expected) = sec.token.as_ref().filter(|_| !peer_envelope) {
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
        assert_eq!(query.status(), StatusCode::OK);
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
}
