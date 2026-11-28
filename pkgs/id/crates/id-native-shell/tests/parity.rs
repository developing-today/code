//! Contract-parity tests for design #788 step 2: the native shell and
//! the terminal shell must agree on (a) the client frames a given
//! logical input produces and (b) the presented text for a given host
//! view. The terminal side of the oracle is the shared frame builders
//! and `WorldSurface::text_projection` — the exact code paths the
//! terminal shell presents through.
//!
//! The durable world-outcome half of the design's parity criterion
//! (same frames into a live host, same journal) is proven against a
//! running host in `tests/live_session.rs`; what is proven here is
//! the frame/presentation equivalence that loop relies on.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id_native_shell::events::{NativeEvent, native_event_frame};
use id_native_shell::render::ScreenGrid;
use id_native_shell::{SessionState, chat_frame, input_frame, paste_frame, resize_frame};
use serde_json::json;

#[test]
fn native_and_terminal_frames_agree_for_the_scripted_inputs() {
    let scripted = [
        (
            NativeEvent::KeyInput(b"ls\r".to_vec()),
            input_frame(b"ls\r"),
        ),
        (
            NativeEvent::Paste("pasted text".to_string()),
            paste_frame("pasted text").unwrap(),
        ),
        (
            NativeEvent::Resize {
                width: 120,
                height: 40,
            },
            resize_frame(120, 40).unwrap(),
        ),
        (
            NativeEvent::Chat("hello world".to_string()),
            chat_frame("hello world"),
        ),
    ];
    for (native, terminal) in scripted {
        assert_eq!(native_event_frame(&native).unwrap(), terminal);
    }
}

#[test]
fn native_presentation_matches_terminal_projection_for_a_screen_view() {
    // One scripted host stream: snapshot, a durable event, then a view
    // carrying a validated Screen surface in the host's normalized
    // cells wire format.
    let mut native = SessionState::default();
    native
        .apply_server_frame(json!({"type": "snapshot", "snapshot": {"sequence": 4}}))
        .unwrap();
    native
        .apply_server_frame(
            json!({"type": "event", "event": {"sequence": 5, "kind": {"kind": "input"}}}),
        )
        .unwrap();
    native
        .apply_server_frame(json!({
            "type": "view", "sequence": 5, "data": "fallback text",
            "surface": {"kind": "screen", "version": 1, "width": 5, "height": 2, "cells": [
                {"symbol": "h", "fg": 7, "bg": 0, "bold": false, "columns": 1},
                {"symbol": "e", "fg": 7, "bg": 0, "bold": false, "columns": 1},
                {"symbol": "l", "fg": 7, "bg": 0, "bold": false, "columns": 1},
                {"symbol": "l", "fg": 7, "bg": 0, "bold": false, "columns": 1},
                {"symbol": "o", "fg": 7, "bg": 0, "bold": false, "columns": 1},
                {"symbol": "w", "fg": 10, "bg": 0, "bold": true, "columns": 1},
                {"symbol": "o", "fg": 10, "bg": 0, "bold": true, "columns": 1},
                {"symbol": "r", "fg": 10, "bg": 0, "bold": true, "columns": 1},
                {"symbol": "l", "fg": 10, "bg": 0, "bold": true, "columns": 1},
                {"symbol": "d", "fg": 10, "bg": 0, "bold": true, "columns": 1}
            ]},
            "view_revision": 0
        }))
        .unwrap();

    let view = native.view().unwrap();
    // The terminal shell presents the surface's text projection; the
    // native grid must project to the same text, row for row.
    let terminal_text = view
        .surface
        .as_ref()
        .expect("validated screen surface")
        .text_projection();
    let grid = ScreenGrid::from_view(view);
    assert_eq!(grid.text_projection(), terminal_text);
    assert_eq!(grid.text_projection(), "hello\nworld");
    // And the styled runs keep the two style groups separate.
    assert_eq!(ScreenGrid::row_runs(&grid.rows[1]).len(), 1);
    assert!(ScreenGrid::row_runs(&grid.rows[1])[0].3);
}

#[test]
fn surface_less_views_fall_back_to_data_on_both_shells() {
    let mut native = SessionState::default();
    native
        .apply_server_frame(
            json!({"type": "view", "sequence": 1, "data": "legacy only", "view_revision": 0}),
        )
        .unwrap();
    let view = native.view().unwrap();
    assert_eq!(view.text_projection(), "legacy only");
    assert_eq!(ScreenGrid::from_view(view).text_projection(), "legacy only");
}
