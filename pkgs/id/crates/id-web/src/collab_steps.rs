//! Server-side contract for collaborative-editing steps (#1618 slice 1).
//!
//! The collab server is the authority for a document's step history:
//! every client replays that history from version 0 on join, and
//! `ProseMirror`'s `receiveTransaction` cannot skip a step in the middle
//! of it. A step the server stores but no client can apply therefore
//! poisons the document for every current and future client.
//!
//! This module is the first, fail-closed line of defence. Before a batch
//! is stored, each step must:
//!
//! - be a JSON object with a `stepType` the editor knows (`replace`,
//!   `replaceAround`, `addMark`, `removeMark`, `addNodeMark`,
//!   `removeNodeMark`, `attr`, `docAttr`);
//! - carry the position fields that step type needs, as non-negative
//!   integers, in the order the step type requires;
//! - name only node and mark types that exist in the schema of the
//!   document's content mode (the rich schema, or the raw code-block
//!   schema), with well-formed slice content;
//! - stay inside the document as it stands at that point in the batch.
//!   The server tracks the document's content size from the base
//!   document and every accepted step, so an out-of-range position is
//!   refused even when the step is otherwise well formed.
//!
//! This does not yet prove that a structurally plausible step applies
//! against the evolving document. Slice validation also remains partial:
//! `list_item` rejects direct text children, but other content-expression
//! violations are deferred. Full server-side application is the later
//! stage of #1618.

use serde_json::{Map, Value};

use super::content_mode::ContentMode;

/// The deepest node nesting accepted inside a step's slice. Real
/// documents nest a few levels; this bounds recursion on hostile input.
const MAX_NODE_DEPTH: usize = 128;

/// Node types of the rich editor schema (`richSchema` in
/// `web/src/editor.ts`: prosemirror-schema-basic, prosemirror-schema-list
/// and the GFM extensions).
const RICH_NODES: &[&str] = &[
    "doc",
    "paragraph",
    "blockquote",
    "horizontal_rule",
    "heading",
    "code_block",
    "text",
    "image",
    "hard_break",
    "ordered_list",
    "bullet_list",
    "list_item",
    "task_list",
    "task_list_item",
    "table",
    "table_row",
    "table_cell",
    "table_header",
    "html_inline",
    "front_matter",
    "footnote_reference",
    "footnote_definition",
];

/// Rich-schema node types with no content (`ProseMirror` leaf nodes,
/// size 1). `text` is sized by its text instead.
const RICH_LEAVES: &[&str] = &[
    "horizontal_rule",
    "image",
    "hard_break",
    "html_inline",
    "front_matter",
    "footnote_reference",
];

/// Mark types of the rich editor schema.
const RICH_MARKS: &[&str] = &["link", "em", "strong", "code", "strikethrough"];

/// Node types of the raw (code-block only) editor schema.
const RAW_NODES: &[&str] = &["doc", "text", "code_block"];

/// The node and mark types a document's editor schema allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepSchema {
    nodes: &'static [&'static str],
    leaves: &'static [&'static str],
    marks: &'static [&'static str],
}

impl StepSchema {
    /// The schema the editor uses for `mode`, or `None` for a mode the
    /// editor never edits (media and binary files).
    #[must_use]
    pub const fn for_mode(mode: ContentMode) -> Option<Self> {
        match mode {
            ContentMode::Rich | ContentMode::Markdown | ContentMode::Plain => Some(Self {
                nodes: RICH_NODES,
                leaves: RICH_LEAVES,
                marks: RICH_MARKS,
            }),
            ContentMode::Raw => Some(Self {
                nodes: RAW_NODES,
                leaves: &[],
                marks: &[],
            }),
            ContentMode::Media(_) | ContentMode::Binary => None,
        }
    }

    fn has_node(&self, name: &str) -> bool {
        self.nodes.contains(&name)
    }

    fn is_leaf(&self, name: &str) -> bool {
        self.leaves.contains(&name)
    }

    fn has_mark(&self, name: &str) -> bool {
        self.marks.contains(&name)
    }
}

/// Why a batch of steps was refused: the index of the first bad step in
/// the batch and what is wrong with it. Nothing in a refused batch is
/// stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRefusal {
    /// Index of the refused step within its batch.
    pub index: usize,
    /// What is wrong with the step.
    pub reason: String,
}

impl std::fmt::Display for StepRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid step {}: {}", self.index, self.reason)
    }
}

/// The content size of a `ProseMirror` document, as `doc.content.size`.
///
/// This sizes the server's base document, which the server produced
/// itself (or read from a stored `ProseMirror` file), so it is
/// permissive: unknown node types count as empty non-leaf nodes and
/// nesting beyond the depth bound counts as nothing.
#[must_use]
pub fn document_content_size(doc: &Value, mode: ContentMode) -> u64 {
    let leaves: &[&str] = match StepSchema::for_mode(mode) {
        Some(schema) => schema.leaves,
        None => &[],
    };
    doc.get("content")
        .and_then(Value::as_array)
        .map_or(0, |content| {
            content
                .iter()
                .map(|node| permissive_node_size(node, leaves, 0))
                .fold(0, u64::saturating_add)
        })
}

fn permissive_node_size(node: &Value, leaves: &[&str], depth: usize) -> u64 {
    if depth > MAX_NODE_DEPTH {
        return 0;
    }
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "text" {
        return node
            .get("text")
            .and_then(Value::as_str)
            .map_or(0, utf16_len);
    }
    if let Some(content) = node.get("content").and_then(Value::as_array) {
        return content
            .iter()
            .map(|child| permissive_node_size(child, leaves, depth + 1))
            .fold(2, u64::saturating_add);
    }
    if leaves.contains(&kind) { 1 } else { 2 }
}

/// Check a batch of steps against the document as it stands, with
/// content size `size`, under `schema`.
///
/// Each step is checked against the size left by the steps before it,
/// so a batch is accepted or refused as a whole. Returns the document's
/// content size after the whole batch.
///
/// # Errors
///
/// Returns the first refused step's index and reason.
pub fn check_steps(schema: StepSchema, size: u64, steps: &[Value]) -> Result<u64, StepRefusal> {
    steps
        .iter()
        .enumerate()
        .try_fold(size, |size, (index, step)| {
            check_step(schema, size, step).map_err(|reason| StepRefusal { index, reason })
        })
}

/// Check one step against a document of content size `size`; returns
/// the content size after it.
fn check_step(schema: StepSchema, size: u64, step: &Value) -> Result<u64, String> {
    let Some(step) = step.as_object() else {
        return Err("step is not a JSON object".to_owned());
    };
    let Some(step_type) = step.get("stepType").and_then(Value::as_str) else {
        return Err("step has no string `stepType`".to_owned());
    };
    match step_type {
        "replace" => {
            let from = position(step, "from")?;
            let to = position(step, "to")?;
            ordered(&[("from", from), ("to", to)], size)?;
            let slice = slice_size(schema, step.get("slice"))?;
            Ok(size - (to - from) + slice)
        }
        "replaceAround" => {
            let from = position(step, "from")?;
            let gap_from = position(step, "gapFrom")?;
            let gap_to = position(step, "gapTo")?;
            let to = position(step, "to")?;
            let insert = position(step, "insert")?;
            ordered(
                &[
                    ("from", from),
                    ("gapFrom", gap_from),
                    ("gapTo", gap_to),
                    ("to", to),
                ],
                size,
            )?;
            let slice = slice_size(schema, step.get("slice"))?;
            if insert > slice {
                return Err(format!(
                    "`insert` {insert} is past the end of the slice (size {slice})"
                ));
            }
            Ok(size - (to - from) + slice + (gap_to - gap_from))
        }
        "addMark" | "removeMark" => {
            let from = position(step, "from")?;
            let to = position(step, "to")?;
            ordered(&[("from", from), ("to", to)], size)?;
            check_mark(schema, step.get("mark"))?;
            Ok(size)
        }
        "addNodeMark" | "removeNodeMark" => {
            node_position(step, size)?;
            check_mark(schema, step.get("mark"))?;
            Ok(size)
        }
        "attr" => {
            node_position(step, size)?;
            attr_name(step)?;
            Ok(size)
        }
        "docAttr" => {
            attr_name(step)?;
            Ok(size)
        }
        other => Err(format!("unknown step type `{other}`")),
    }
}

fn position(step: &Map<String, Value>, field: &str) -> Result<u64, String> {
    step.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("`{field}` must be a non-negative integer"))
}

/// Positions that must not decrease, all inside a document of content
/// size `size`.
fn ordered(fields: &[(&str, u64)], size: u64) -> Result<(), String> {
    let mut previous: Option<(&str, u64)> = None;
    for &(name, value) in fields {
        if let Some((before, before_value)) = previous
            && value < before_value
        {
            return Err(format!(
                "`{name}` {value} is before `{before}` {before_value}"
            ));
        }
        previous = Some((name, value));
    }
    if let Some(&(name, value)) = fields.last()
        && value > size
    {
        return Err(format!(
            "`{name}` {value} is past the end of the document (size {size})"
        ));
    }
    Ok(())
}

/// The `pos` of a step that targets the node starting there.
fn node_position(step: &Map<String, Value>, size: u64) -> Result<u64, String> {
    let pos = position(step, "pos")?;
    if pos >= size {
        return Err(format!(
            "`pos` {pos} does not point at a node in the document (size {size})"
        ));
    }
    Ok(pos)
}

fn attr_name(step: &Map<String, Value>) -> Result<(), String> {
    match step.get("attr").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => Ok(()),
        _ => Err("`attr` must be a non-empty string".to_owned()),
    }
}

fn check_mark(schema: StepSchema, mark: Option<&Value>) -> Result<(), String> {
    let Some(mark) = mark.and_then(Value::as_object) else {
        return Err("`mark` must be a JSON object".to_owned());
    };
    let Some(kind) = mark.get("type").and_then(Value::as_str) else {
        return Err("mark has no string `type`".to_owned());
    };
    if !schema.has_mark(kind) {
        return Err(format!("unknown mark type `{kind}`"));
    }
    match mark.get("attrs") {
        None | Some(Value::Object(_) | Value::Null) => Ok(()),
        Some(_) => Err(format!("mark `{kind}` has non-object `attrs`")),
    }
}

/// The size of a step's slice (`Slice.size`): its content's size less
/// its open depths. A missing or null slice is the empty slice.
fn slice_size(schema: StepSchema, slice: Option<&Value>) -> Result<u64, String> {
    let slice = match slice {
        None | Some(Value::Null) => return Ok(0),
        Some(Value::Object(slice)) => slice,
        Some(_) => return Err("`slice` must be a JSON object".to_owned()),
    };
    let content = match slice.get("content") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(nodes)) => fragment_size(schema, nodes, 0)?,
        Some(_) => return Err("slice `content` must be an array".to_owned()),
    };
    let open_start = optional_depth(slice, "openStart")?;
    let open_end = optional_depth(slice, "openEnd")?;
    content
        .checked_sub(open_start)
        .and_then(|rest| rest.checked_sub(open_end))
        .ok_or_else(|| {
            format!(
                "slice open depths ({open_start} + {open_end}) exceed its content size {content}"
            )
        })
}

fn optional_depth(slice: &Map<String, Value>, field: &str) -> Result<u64, String> {
    match slice.get(field) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("slice `{field}` must be a non-negative integer")),
    }
}

fn fragment_size(schema: StepSchema, nodes: &[Value], depth: usize) -> Result<u64, String> {
    nodes.iter().try_fold(0u64, |total, node| {
        node_size(schema, node, depth).map(|size| total.saturating_add(size))
    })
}

/// The size of one node inside a slice, checked against the schema.
fn node_size(schema: StepSchema, node: &Value, depth: usize) -> Result<u64, String> {
    if depth >= MAX_NODE_DEPTH {
        return Err(format!("slice nests deeper than {MAX_NODE_DEPTH} nodes"));
    }
    let Some(node) = node.as_object() else {
        return Err("slice node is not a JSON object".to_owned());
    };
    let Some(kind) = node.get("type").and_then(Value::as_str) else {
        return Err("slice node has no string `type`".to_owned());
    };
    if kind == "doc" || !schema.has_node(kind) {
        return Err(format!("unknown node type `{kind}`"));
    }
    match node.get("marks") {
        None | Some(Value::Null) => {}
        Some(Value::Array(marks)) => {
            for mark in marks {
                check_mark(schema, Some(mark))?;
            }
        }
        Some(_) => return Err(format!("node `{kind}` has non-array `marks`")),
    }
    match node.get("attrs") {
        None | Some(Value::Object(_) | Value::Null) => {}
        Some(_) => return Err(format!("node `{kind}` has non-object `attrs`")),
    }
    let content = match node.get("content") {
        None | Some(Value::Null) => None,
        Some(Value::Array(children)) => Some(children.as_slice()),
        Some(_) => return Err(format!("node `{kind}` has non-array `content`")),
    };
    // `editor.ts` declares `list_item` content as `paragraph block*`.
    // A bare text child cannot be produced by a valid rich-schema item.
    if kind == "list_item"
        && content.is_some_and(|children| {
            children
                .iter()
                .any(|child| child.get("type").and_then(Value::as_str) == Some("text"))
        })
    {
        return Err("list_item content cannot contain a direct text child".to_owned());
    }
    if kind == "text" {
        if content.is_some_and(|children| !children.is_empty()) {
            return Err("text node has content".to_owned());
        }
        return match node.get("text").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => Ok(utf16_len(text)),
            _ => Err("text node needs a non-empty string `text`".to_owned()),
        };
    }
    if schema.is_leaf(kind) {
        if content.is_some_and(|children| !children.is_empty()) {
            return Err(format!("leaf node `{kind}` has content"));
        }
        return Ok(1);
    }
    let inner = match content {
        Some(children) => fragment_size(schema, children, depth + 1)?,
        None => 0,
    };
    Ok(inner.saturating_add(2))
}

/// `ProseMirror` measures text in UTF-16 code units (JavaScript string
/// length).
fn utf16_len(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rich() -> StepSchema {
        StepSchema::for_mode(ContentMode::Markdown).unwrap()
    }

    fn raw() -> StepSchema {
        StepSchema::for_mode(ContentMode::Raw).unwrap()
    }

    /// `<p>hello world</p>`: content size 13.
    fn hello_doc() -> Value {
        json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "hello world"}]}]
        })
    }

    #[test]
    fn document_size_matches_prosemirror_sizes() {
        assert_eq!(
            document_content_size(&hello_doc(), ContentMode::Markdown),
            13
        );
        // An empty paragraph is 2; a leaf is 1; text counts UTF-16 units.
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph"},
            {"type": "horizontal_rule"},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "é😀"},
                {"type": "hard_break"},
                {"type": "image", "attrs": {"src": "x"}}
            ]}
        ]});
        assert_eq!(
            document_content_size(&doc, ContentMode::Rich),
            2 + 1 + (2 + 3 + 1 + 1)
        );
        assert_eq!(
            document_content_size(&json!({"type": "doc"}), ContentMode::Raw),
            0
        );
    }

    #[test]
    fn valid_steps_pass_and_track_size() {
        let size = document_content_size(&hello_doc(), ContentMode::Markdown);
        let steps = vec![
            // Type "!" at the end of the text.
            json!({"stepType": "replace", "from": 12, "to": 12,
                   "slice": {"content": [{"type": "text", "text": "!"}]}}),
            // Bold "hello".
            json!({"stepType": "addMark", "from": 1, "to": 6, "mark": {"type": "strong"}}),
            // Split the paragraph after "hello": a slice of two open
            // paragraphs, size 2.
            json!({"stepType": "replace", "from": 6, "to": 6,
                   "slice": {"content": [{"type": "paragraph"}, {"type": "paragraph"}],
                             "openStart": 1, "openEnd": 1}}),
            // Delete " world!", now in the second paragraph at 8..15.
            json!({"stepType": "replace", "from": 8, "to": 15}),
            // Wrap the first paragraph in a blockquote.
            json!({"stepType": "replaceAround", "from": 0, "to": 7, "gapFrom": 0,
                   "gapTo": 7, "insert": 1, "structure": true,
                   "slice": {"content": [{"type": "blockquote"}]}}),
            json!({"stepType": "attr", "pos": 0, "attr": "x", "value": 1}),
            json!({"stepType": "docAttr", "attr": "x", "value": 1}),
        ];
        // 13 +1 (!) +2 (split) -7 (delete) +2 (blockquote).
        assert_eq!(check_steps(rich(), size, &steps), Ok(11));
    }

    #[test]
    fn issue_1618_probe_steps_are_refused() {
        let size = document_content_size(&hello_doc(), ContentMode::Markdown);
        let refused = |step: Value| check_steps(rich(), size, &[step]).unwrap_err().reason;
        assert!(
            refused(json!({"stepType": "bogusType", "from": 1, "to": 2})).contains("bogusType")
        );
        assert!(
            refused(json!({"stepType": "replace", "from": 1, "to": 999_999}))
                .contains("past the end")
        );
        assert!(
            refused(json!({"stepType": "replace", "from": 1, "to": 1,
                           "slice": {"content": [{"type": "no_such_node"}]}}))
            .contains("no_such_node")
        );
        for non_object in [json!(42), json!(null), json!("hi")] {
            assert!(refused(non_object).contains("not a JSON object"));
        }
    }

    #[test]
    fn malformed_fields_are_refused() {
        let size = 13;
        let cases = [
            json!({"from": 1, "to": 2}),
            json!({"stepType": "replace", "from": -1, "to": 2}),
            json!({"stepType": "replace", "from": 1.5, "to": 2}),
            json!({"stepType": "replace", "from": 3, "to": 2}),
            json!({"stepType": "replace", "from": 1}),
            json!({"stepType": "replace", "from": 1, "to": 1, "slice": []}),
            json!({"stepType": "replace", "from": 1, "to": 1, "slice": {"content": {}}}),
            json!({"stepType": "replace", "from": 1, "to": 1,
                   "slice": {"content": [{"type": "paragraph"}], "openStart": 2, "openEnd": 1}}),
            json!({"stepType": "replace", "from": 1, "to": 1,
                   "slice": {"content": [{"type": "text", "text": ""}]}}),
            json!({"stepType": "replace", "from": 1, "to": 1,
                   "slice": {"content": [{"type": "hard_break", "content": [{"type": "text", "text": "x"}]}]}}),
            json!({"stepType": "replace", "from": 1, "to": 1,
                   "slice": {"content": [{"type": "doc"}]}}),
            json!({"stepType": "replace", "from": 1, "to": 1,
                   "slice": {"content": [{"type": "text", "text": "x", "marks": [{"type": "blink"}]}]}}),
            json!({"stepType": "replaceAround", "from": 0, "to": 13, "gapFrom": 5,
                   "gapTo": 4, "insert": 0}),
            json!({"stepType": "replaceAround", "from": 0, "to": 13, "gapFrom": 0,
                   "gapTo": 13, "insert": 3, "slice": {"content": [{"type": "blockquote"}]}}),
            json!({"stepType": "addMark", "from": 1, "to": 6, "mark": {"type": "blink"}}),
            json!({"stepType": "addMark", "from": 1, "to": 6}),
            json!({"stepType": "addMark", "from": 1, "to": 14, "mark": {"type": "em"}}),
            json!({"stepType": "addNodeMark", "pos": 13, "mark": {"type": "em"}}),
            json!({"stepType": "attr", "pos": 0, "value": 1}),
            json!({"stepType": "docAttr", "attr": "", "value": 1}),
        ];
        for step in cases {
            assert!(
                check_steps(rich(), size, std::slice::from_ref(&step)).is_err(),
                "accepted {step}"
            );
        }
    }

    #[test]
    fn a_batch_is_checked_against_the_size_its_own_steps_leave() {
        // Deleting the whole text makes a later step at the old end out
        // of range, and the refusal names that later step.
        let steps = vec![
            json!({"stepType": "replace", "from": 1, "to": 12}),
            json!({"stepType": "replace", "from": 12, "to": 12,
                   "slice": {"content": [{"type": "text", "text": "!"}]}}),
        ];
        let refusal = check_steps(rich(), 13, &steps).unwrap_err();
        assert_eq!(refusal.index, 1);
        assert!(refusal.to_string().starts_with("Invalid step 1:"));
    }

    #[test]
    fn raw_mode_refuses_rich_content_and_marks() {
        let doc = json!({"type": "doc", "content": [{"type": "code_block", "content": [{"type": "text", "text": "x = 1"}]}]});
        let size = document_content_size(&doc, ContentMode::Raw);
        assert_eq!(size, 7);
        let typed = json!({"stepType": "replace", "from": 6, "to": 6,
                           "slice": {"content": [{"type": "text", "text": ";"}]}});
        assert_eq!(check_steps(raw(), size, &[typed]), Ok(8));
        let heading = json!({"stepType": "replace", "from": 7, "to": 7,
                             "slice": {"content": [{"type": "heading"}]}});
        assert!(check_steps(raw(), size, &[heading]).is_err());
        let bold = json!({"stepType": "addMark", "from": 1, "to": 2, "mark": {"type": "strong"}});
        assert!(check_steps(raw(), size, &[bold]).is_err());
        assert!(StepSchema::for_mode(ContentMode::Binary).is_none());
    }

    #[test]
    fn deep_slices_are_refused_not_recursed() {
        let mut node = json!({"type": "text", "text": "x"});
        for _ in 0..=MAX_NODE_DEPTH {
            node = json!({"type": "blockquote", "content": [node]});
        }
        let step = json!({"stepType": "replace", "from": 0, "to": 0, "slice": {"content": [node]}});
        let refusal = check_steps(rich(), 2, &[step]).unwrap_err();
        assert!(refusal.reason.contains("deeper"));
    }
}
