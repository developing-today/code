//! GitHub's App endpoints, read with an App JWT only. Pages are followed
//! through the `Link` header until it names no `next`, never by counting.

use crate::link::next_link;
use crate::sync::{AppMeta, GitHubApi, Installation};
use serde_json::Value;

pub const GITHUB_API: &str = "https://api.github.com";

/// Hard stop against a server that keeps naming a next page.
const MAX_PAGES: usize = 1000;

/// Follow `Link: rel="next"` from `first` until the last page, returning
/// every page's items in order.
pub fn collect_pages<F>(first: &str, mut fetch: F) -> Result<Vec<Value>, String>
where
    F: FnMut(&str) -> Result<(Value, Option<String>), String>,
{
    let mut items = Vec::new();
    let mut url = first.to_string();
    for _ in 0..MAX_PAGES {
        let (page, next) = fetch(&url)?;
        match page {
            Value::Array(list) => items.extend(list),
            _ => return Err("paginated body is not an array".to_string()),
        }
        match next {
            Some(n) => url = n,
            None => return Ok(items),
        }
    }
    Err(format!("more than {MAX_PAGES} pages; refusing to continue"))
}

pub fn parse_app(body: &Value) -> Result<AppMeta, String> {
    let slug = body
        .get("slug")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("GET /app: no slug")?;
    Ok(AppMeta {
        slug: slug.to_string(),
        app_id: body
            .get("id")
            .and_then(Value::as_i64)
            .ok_or("GET /app: no id")?,
        name: body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(slug)
            .to_string(),
        owner: body
            .get("owner")
            .and_then(|o| o.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        permissions: body
            .get("permissions")
            .cloned()
            .unwrap_or(Value::Object(Default::default())),
        events: body
            .get("events")
            .cloned()
            .unwrap_or(Value::Array(Vec::new())),
    })
}

pub fn parse_installation(entry: &Value) -> Option<Installation> {
    let account = entry.get("account")?;
    Some(Installation {
        installation_id: entry.get("id").and_then(Value::as_i64)?,
        account_login: account.get("login").and_then(Value::as_str)?.to_string(),
        account_type: account
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        repository_selection: entry
            .get("repository_selection")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        permissions: entry
            .get("permissions")
            .cloned()
            .unwrap_or(Value::Object(Default::default())),
    })
}

/// The live client: ureq over the GitHub REST API.
pub struct HttpGitHub {
    agent: ureq::Agent,
    base: String,
}

impl Default for HttpGitHub {
    fn default() -> Self {
        Self::new(GITHUB_API)
    }
}

impl HttpGitHub {
    pub fn new(base: &str) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        HttpGitHub {
            agent,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn get(&self, jwt: &str, url: &str) -> Result<(Value, Option<String>), String> {
        let mut resp = self
            .agent
            .get(url)
            .header("Authorization", &format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "fleet-orgs")
            .call()
            .map_err(|e| format!("github transport error: {e}"))?;
        let status = resp.status().as_u16();
        let next = resp
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .and_then(next_link);
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("github body read error: {e}"))?;
        if status != 200 {
            return Err(format!("GET {url}: HTTP {status}"));
        }
        let body =
            serde_json::from_str(&text).map_err(|_| format!("GET {url}: unparseable body"))?;
        Ok((body, next))
    }
}

impl GitHubApi for HttpGitHub {
    fn app(&self, jwt: &str) -> Result<AppMeta, String> {
        let (body, _) = self.get(jwt, &format!("{}/app", self.base))?;
        parse_app(&body)
    }

    fn installations(&self, jwt: &str) -> Result<Vec<Installation>, String> {
        let first = format!("{}/app/installations?per_page=100", self.base);
        let entries = collect_pages(&first, |url| self.get(jwt, url))?;
        entries
            .iter()
            .map(|e| {
                parse_installation(e).ok_or_else(|| "installation entry missing fields".to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn follows_every_link_page_in_order() {
        let pages = |url: &str| -> Result<(Value, Option<String>), String> {
            match url {
                "p1" => Ok((json!([{"id": 1}]), Some("p2".into()))),
                "p2" => Ok((json!([{"id": 2}]), Some("p3".into()))),
                "p3" => Ok((json!([{"id": 3}]), None)),
                other => Err(format!("unexpected {other}")),
            }
        };
        let items = collect_pages("p1", pages).unwrap();
        let ids: Vec<i64> = items.iter().map(|v| v["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn a_non_array_page_is_an_error() {
        let r = collect_pages("p1", |_| Ok((json!({"total_count": 0}), None)));
        assert!(r.is_err());
    }

    #[test]
    fn parses_an_installation_with_its_permissions() {
        let entry = json!({
            "id": 42,
            "account": {"login": "acme", "type": "Organization"},
            "repository_selection": "all",
            "permissions": {"contents": "write", "issues": "read"}
        });
        let inst = parse_installation(&entry).unwrap();
        assert_eq!(inst.installation_id, 42);
        assert_eq!(inst.account_login, "acme");
        assert_eq!(inst.account_type, "Organization");
        assert_eq!(inst.permissions["contents"], "write");
    }

    #[test]
    fn parses_app_identity_permissions_events_and_owner() {
        let app = parse_app(&json!({
            "slug": "fleet-app",
            "id": 42,
            "name": "Fleet App",
            "owner": {"login": "developing-today"},
            "permissions": {"contents": "write"},
            "events": ["push"]
        }))
        .unwrap();
        assert_eq!(app.slug, "fleet-app");
        assert_eq!(app.owner, "developing-today");
        assert_eq!(app.permissions["contents"], "write");
        assert_eq!(app.events, json!(["push"]));
    }

    #[test]
    fn an_entry_without_an_id_or_account_is_skipped_as_invalid() {
        assert!(parse_installation(&json!({"id": 1})).is_none());
        assert!(parse_installation(&json!({"account": {"login": "a"}})).is_none());
    }

    #[test]
    fn app_jwt_installation_listing_follows_pages_not_oauth_org_reads() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for page in 1..=2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let n = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let first = request.lines().next().unwrap();
                assert!(first.starts_with("GET "));
                assert!(first.contains("/app/installations?per_page=100"));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer app-jwt")
                );
                let values: Vec<Value> = (0..if page == 1 { 32 } else { 1 })
                    .map(|i| json!({
                        "id": (page - 1) * 32 + i + 1,
                        "account": {"login": format!("org{}", (page - 1) * 32 + i + 1), "type": "Organization"},
                        "repository_selection": "all",
                        "permissions": {"contents": "read"}
                    }))
                    .collect();
                let body = serde_json::to_string(&values).unwrap();
                let link = if page == 1 {
                    format!(
                        "Link: <http://{addr}/app/installations?per_page=100&page=2>; rel=\"next\"\r\n"
                    )
                } else {
                    String::new()
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{link}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });

        let github = HttpGitHub::new(&format!("http://{addr}"));
        let installations = github.installations("app-jwt").unwrap();
        server.join().unwrap();
        assert_eq!(
            installations.len(),
            33,
            "oracle: all 33 fixture org installations are covered"
        );
        assert_eq!(installations[32].account_login, "org33");
    }
}
