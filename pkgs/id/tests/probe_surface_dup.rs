// Probe v2: parse_surface duplicate JSON keys (raw strings; json! cannot express dups)
// Control chars are expressed as JSON escapes (valid JSON) so the document parses.
use id::world_presentation::{parse_surface, WorldSurface};

#[test]
fn probe_surface_dup() {
    // Controls: single-key forms behave as documented.
    let ok_text = parse_surface(r#"{"kind":"text","version":1,"lines":["hi"]}"#);
    assert!(matches!(ok_text, Ok(Some(WorldSurface::Text(_)))), "control text ok: {ok_text:?}");
    let bad_version = parse_surface(r#"{"kind":"text","version":99,"lines":["hi"]}"#);
    assert!(bad_version.is_err(), "control text version 99 must err: {bad_version:?}");
    let bad_width = parse_surface(r#"{"kind":"text","version":1,"width":0,"lines":["hi"]}"#);
    assert!(bad_width.is_err(), "control text width 0 must err: {bad_width:?}");
    let bad_lines = parse_surface(r#"{"kind":"text","version":1,"lines":["\u001b[31mred"]}"#);
    assert!(bad_lines.is_err(), "control text escaped control-char line must err: {bad_lines:?}");
    let ok_screen = parse_surface(r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["hi"]}"#);
    assert!(matches!(ok_screen, Ok(Some(WorldSurface::Screen(_)))), "control screen ok: {ok_screen:?}");
    let bad_screen_version = parse_surface(r#"{"kind":"screen","version":99,"width":2,"height":1,"rows":["hi"]}"#);
    assert!(bad_screen_version.is_err(), "control screen version 99 must err: {bad_screen_version:?}");
    let bad_rows = parse_surface(r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["toolong"]}"#);
    assert!(bad_rows.is_err(), "control screen bad rows must err: {bad_rows:?}");

    // Duplicate-key forms: the first (invalid) value is hidden by the second.
    let dup_version_text = parse_surface(r#"{"kind":"text","version":99,"version":1,"lines":["hi"]}"#);
    let dup_width = parse_surface(r#"{"kind":"text","version":1,"width":0,"width":80,"lines":["hi"]}"#);
    let dup_lines = parse_surface(r#"{"kind":"text","version":1,"lines":["\u001b[31mred"],"lines":["clean"]}"#);
    let dup_version_screen = parse_surface(r#"{"kind":"screen","version":99,"version":1,"width":2,"height":1,"rows":["hi"]}"#);
    let dup_rows = parse_surface(r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["toolong"],"rows":["hi"]}"#);
    let dup_kind = parse_surface(r#"{"kind":"screen","kind":"text","version":1,"lines":["hi"]}"#);

    println!("dup_version_text => {dup_version_text:?}");
    println!("dup_width => {dup_width:?}");
    println!("dup_lines => {dup_lines:?}");
    println!("dup_version_screen => {dup_version_screen:?}");
    println!("dup_rows => {dup_rows:?}");
    println!("dup_kind => {dup_kind:?}");

    assert!(matches!(dup_version_text, Ok(Some(WorldSurface::Text(_)))), "dup version text accepted");
    assert!(matches!(dup_width, Ok(Some(WorldSurface::Text(_)))), "dup width accepted");
    assert!(matches!(dup_lines, Ok(Some(WorldSurface::Text(_)))), "dup lines accepted");
    assert!(matches!(dup_version_screen, Ok(Some(WorldSurface::Screen(_)))), "dup version screen accepted");
    assert!(matches!(dup_rows, Ok(Some(WorldSurface::Screen(_)))), "dup rows accepted");
    assert!(matches!(dup_kind, Ok(Some(WorldSurface::Text(_)))), "dup kind accepted");
}
