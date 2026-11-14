// Probe: does Petnames::open replay records that Petnames::set refuses?
// Updated for the load-time validation fix (issue #5225): open must now
// fail loud on the first journal record set() would refuse, instead of
// loading it verbatim.
use id_core::petname::Petnames;

#[test]
#[allow(clippy::unwrap_used)]
fn probe_open_rejects_names_set_would_refuse() {
    let dir = std::env::temp_dir().join(format!("petname-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("petnames.jsonl");
    // Two contacts with fold-equal names (set() refuses the second as Conflict),
    // plus a name with a format char (U+200B) and an overlong name (>64 chars).
    let long = "x".repeat(80);
    let journal = format!(
        "{{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-a\",\"name\":\"PayPal\"}}\n\
         {{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-b\",\"name\":\"paypal\"}}\n\
         {{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-c\",\"name\":\"Evil\u{200b}Name\"}}\n\
         {{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-d\",\"name\":\"{long}\"}}\n"
    );
    std::fs::write(&path, journal).unwrap();
    let error =
        Petnames::open(&path).expect_err("journal with refused-at-write records must not load");
    let message = format!("{error:#}");
    assert!(
        message.contains("line 2"),
        "error names the first invalid journal line: {message}"
    );
    assert!(
        message.contains("fold uniqueness"),
        "error names the violated invariant: {message}"
    );
}
