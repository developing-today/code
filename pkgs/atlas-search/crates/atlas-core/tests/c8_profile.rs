//! C8-M profile core tests (developing-today/code#9576, #12919).
//!
//! Oracle: the merged C8 design,
//! `doc/2026-11-22T01-48-21Z_design_c8_atlas_ingestion_investigation`,
//! §2 (corpus metadata envelope: `url_parts` parsed from the canonical
//! source URL; credentials and secret query parameters are discarded)
//! and §3 (typed links from a closed vocabulary; explicit links carry
//! `evidence: explicit` and no confidence), plus the page metadata
//! schema in #9576 (URL exploded into base name, path, file name, last
//! path part, port, media type; is-HTML / is-video flags).

use atlas_core::c8_profile::{
    C8Page, C8Source, LinkKind, PageKind, UrlParts, extract_links, sanitize_url,
};
use proptest::prelude::*;

#[test]
fn url_parts_explode_a_full_url() {
    let parts = UrlParts::parse(
        "https://user:pass@Reports.Example.com:8443/a/b/incident-2026.md?view=full",
    )
    .expect("absolute URL parses");
    assert_eq!(parts.scheme, "https");
    assert_eq!(parts.host, "reports.example.com");
    assert_eq!(parts.port, Some(8443));
    assert_eq!(parts.path, "/a/b/incident-2026.md");
    assert_eq!(parts.file_name.as_deref(), Some("incident-2026.md"));
    assert_eq!(parts.last_path_part.as_deref(), Some("incident-2026.md"));
    assert_eq!(parts.base_name.as_deref(), Some("incident-2026"));
    assert_eq!(parts.extension.as_deref(), Some("md"));
    assert_eq!(parts.media_type(), Some("text/markdown"));
    assert!(!parts.is_html());
    assert!(!parts.is_video());
}

#[test]
fn sanitize_drops_credentials_secret_params_and_fragment() {
    let raw = "https://user:pass@reports.example.com:8443/a/b/incident-2026.md\
               ?token=abc123&view=full&api_key=zzz&sig=deadbeef#section-2";
    assert_eq!(
        sanitize_url(raw).as_deref(),
        Some("https://reports.example.com:8443/a/b/incident-2026.md?view=full"),
    );
}

#[test]
fn sanitize_keeps_benign_params_and_drops_lone_secret_query() {
    assert_eq!(
        sanitize_url("https://example.com/p?page=2&sort=asc").as_deref(),
        Some("https://example.com/p?page=2&sort=asc"),
    );
    assert_eq!(
        sanitize_url("https://example.com/p?session_id=only").as_deref(),
        Some("https://example.com/p"),
    );
}

#[test]
fn media_type_and_flags_come_from_the_extension() {
    let html = UrlParts::parse("https://example.com/reports/today.html").unwrap();
    assert_eq!(html.media_type(), Some("text/html"));
    assert!(html.is_html());
    assert!(!html.is_video());

    let video = UrlParts::parse("https://example.com/clips/walkthrough.mp4").unwrap();
    assert_eq!(video.media_type(), Some("video/mp4"));
    assert!(video.is_video());
    assert!(!video.is_html());

    let unknown = UrlParts::parse("https://example.com/blob/data.zzz").unwrap();
    assert_eq!(unknown.media_type(), None);
    assert!(!unknown.is_html());
    assert!(!unknown.is_video());
}

#[test]
fn directory_and_root_urls_have_no_file_name() {
    let dir = UrlParts::parse("https://example.com/reports/").unwrap();
    assert_eq!(dir.file_name, None);
    assert_eq!(dir.last_path_part.as_deref(), Some("reports"));
    assert_eq!(dir.base_name, None);

    let root = UrlParts::parse("https://example.com").unwrap();
    assert_eq!(root.path, "/");
    assert_eq!(root.file_name, None);
    assert_eq!(root.last_path_part, None);
    assert_eq!(root.port, None);
}

#[test]
fn non_absolute_urls_do_not_parse() {
    assert_eq!(UrlParts::parse("/a/b/page.md"), None);
    assert_eq!(UrlParts::parse("example.com/page.md"), None);
    assert_eq!(sanitize_url("not a url"), None);
}

#[test]
fn extract_links_classifies_github_shapes_and_dedupes() {
    let body = "\
# Incident notes

Root cause tracked in [the issue](https://github.com/developing-today/code/issues/9576).
Fix landed via https://github.com/developing-today/code/pull/12290 yesterday.
The introducing change is [this commit](https://github.com/developing-today/code/commit/cdca7105eb6bcc979a70771984622977a34b8094).
Background: [the runbook](https://example.com/runbooks/atlas.html) and
[sibling page](sibling.md). Duplicate of the issue link:
[again](https://github.com/developing-today/code/issues/9576).
Contact [ops](mailto:ops@example.com) with questions.
";
    let links = extract_links(body);
    let got: Vec<(LinkKind, &str)> = links
        .iter()
        .map(|l| (l.kind, l.target_id.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (
                LinkKind::MentionsIssue,
                "https://github.com/developing-today/code/issues/9576"
            ),
            (
                LinkKind::MentionsPullRequest,
                "https://github.com/developing-today/code/pull/12290"
            ),
            (
                LinkKind::MentionsCommit,
                "https://github.com/developing-today/code/commit/cdca7105eb6bcc979a70771984622977a34b8094"
            ),
            (
                LinkKind::LinksPage,
                "https://example.com/runbooks/atlas.html"
            ),
            (LinkKind::LinksPage, "sibling.md"),
        ],
    );
}

#[test]
fn extracted_links_are_explicit_without_confidence() {
    let links = extract_links("[i](https://github.com/o/r/issues/3)");
    assert_eq!(links.len(), 1);
    let json = serde_json::to_value(&links[0]).unwrap();
    assert_eq!(json["kind"], "mentions_issue");
    assert_eq!(json["evidence"], "explicit");
    assert!(
        json.get("confidence").is_none(),
        "explicit links carry no confidence: {json}"
    );
    assert!(json.get("source_locator").is_none());
}

#[test]
fn profile_json_matches_the_design_envelope_shape() {
    let source = C8Source::new(
        "report_page",
        "incident-2026-11-22",
        "rev-42",
        "https://user:pass@reports.example.com/incidents/2026.md?token=abc",
        "2026-11-22T10:00:00Z",
        None,
    )
    .expect("source URL sanitizes");
    // The retained canonical URL has neither credentials nor the
    // secret query parameter (design §2).
    assert_eq!(source.url, "https://reports.example.com/incidents/2026.md");
    let source_json = serde_json::to_value(&source).unwrap();
    assert_eq!(source_json["kind"], "report_page");
    assert_eq!(source_json["revision"], "rev-42");
    assert!(source_json.get("modified_at").is_none());

    let page = C8Page::from_url(
        PageKind::IncidentReport,
        "https://reports.example.com/incidents/2026.md",
    )
    .expect("page URL parses");
    let page_json = serde_json::to_value(&page).unwrap();
    assert_eq!(page_json["kind"], "incident_report");
    assert_eq!(page_json["media_type"], "text/markdown");
    assert_eq!(page_json["url_parts"]["scheme"], "https");
    assert_eq!(page_json["url_parts"]["host"], "reports.example.com");
    assert_eq!(page_json["url_parts"]["path"], "/incidents/2026.md");
    assert_eq!(page_json["generated_by"], serde_json::json!([]));
    assert_eq!(page_json["graph_libraries"], serde_json::json!([]));
}

prop_compose! {
    fn arb_url()(
        host in "[a-z]{1,8}\\.(com|org|net)",
        user in prop::option::of("[a-z]{3,6}"),
        params in prop::collection::vec(
            (prop::sample::select(vec![
                "token", "access_token", "api_key", "secret", "password",
                "sig", "page", "view", "q", "utm_source",
            ]), "[a-z0-9]{1,6}"),
            0..6,
        ),
    ) -> String {
        let authority = match &user {
            Some(u) => format!("{u}:pw@{host}"),
            None => host,
        };
        let query = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        if query.is_empty() {
            format!("https://{authority}/r/page.md")
        } else {
            format!("https://{authority}/r/page.md?{query}")
        }
    }
}

proptest! {
    /// Design §2 invariant: sanitization never retains credentials or
    /// a secret query parameter, benign parameters survive, and
    /// sanitizing an already-canonical URL changes nothing.
    #[test]
    fn sanitize_never_leaks_and_is_idempotent(url in arb_url()) {
        let clean = sanitize_url(&url).expect("generated URL sanitizes");
        prop_assert!(
            !clean.contains('@'),
            "credentials survived sanitization: {clean}"
        );
        for secret in ["token=", "access_token=", "api_key=", "secret=", "password=", "sig="] {
            prop_assert!(
                !clean.contains(secret),
                "secret parameter {secret} survived: {clean}"
            );
        }
        if url.contains("page=") {
            prop_assert!(clean.contains("page="), "benign parameter lost: {clean}");
        }
        let twice = sanitize_url(&clean).expect("canonical URL re-sanitizes");
        prop_assert_eq!(twice, clean);
    }
}
