//! Fork extension tests (#5753, Packet A): annotation lists on every
//! part, property-key annotations, children-block annotations, and
//! the `limit.kdl` shapes that motivated the fork. Spec-shaped
//! documents must behave exactly as upstream.

use kdl::{KdlDocument, KdlEntry, KdlNode};

fn parse(doc: &str) -> KdlDocument {
    doc.parse().expect("document should parse")
}

fn annotation_values(node: &KdlNode) -> Vec<&str> {
    node.annotations().iter().map(|a| a.value()).collect()
}

fn entry_annotation_values(entry: &KdlEntry) -> Vec<&str> {
    entry.annotations().iter().map(|a| a.value()).collect()
}

#[test]
fn multiple_annotations_on_node_name() {
    let doc = parse("(a)(b)node\n");
    let node = &doc.nodes()[0];
    assert_eq!(annotation_values(node), vec!["a", "b"]);
    // The upstream single-annotation accessor sees the first one.
    assert_eq!(node.ty().map(|t| t.value()), Some("a"));
    assert_eq!(doc.to_string(), "(a)(b)node\n");
}

#[test]
fn multiple_annotations_on_values() {
    let doc = parse("node (a)(b)1 key=(c)(d)2\n");
    let node = &doc.nodes()[0];
    let arg = &node.entries()[0];
    assert_eq!(entry_annotation_values(arg), vec!["a", "b"]);
    assert_eq!(arg.ty().map(|t| t.value()), Some("a"));
    let prop = &node.entries()[1];
    assert_eq!(prop.name().map(|n| n.value()), Some("key"));
    assert_eq!(entry_annotation_values(prop), vec!["c", "d"]);
    assert_eq!(doc.to_string(), "node (a)(b)1 key=(c)(d)2\n");
}

#[test]
fn prop_key_annotations() {
    let doc = parse("node (author)name=\"Jonathan\" plain=1\n");
    let node = &doc.nodes()[0];
    let prop = &node.entries()[0];
    assert_eq!(prop.name().map(|n| n.value()), Some("name"));
    assert_eq!(
        prop.key_annotations()
            .iter()
            .map(|a| a.value())
            .collect::<Vec<_>>(),
        vec!["author"]
    );
    // A key annotation is not a value annotation.
    assert!(prop.annotations().is_empty());
    assert_eq!(node.entries()[1].key_annotations(), &[]);
    assert_eq!(doc.to_string(), "node (author)name=\"Jonathan\" plain=1\n");

    // Standalone entry parsing carries key annotations too.
    let entry: KdlEntry = "(k)key=1".parse().expect("entry should parse");
    assert_eq!(entry.key_annotations().len(), 1);
    assert_eq!(entry.key_annotations()[0].value(), "k");
}

#[test]
fn children_annotations() {
    let doc = parse("node (ann) {\n    child\n}\n");
    let node = &doc.nodes()[0];
    assert_eq!(
        node.children_annotations()
            .iter()
            .map(|a| a.value())
            .collect::<Vec<_>>(),
        vec!["ann"]
    );
    assert!(node.children().is_some());
    // Rendering re-parses to the same AST (whitespace placement
    // around children annotations is normalized in Packet A; see
    // FORK.md).
    let rendered = doc.to_string();
    let reparsed = parse(&rendered);
    assert_eq!(reparsed, doc);
}

#[test]
fn limit_kdl_shapes() {
    // The triples-as-annotations shapes from pkgs/mappy/migrations/limit.kdl
    // (#5753 §1.1). Parameterized payloads are quoted identifiers
    // under KDL 2.0 identifier rules (`#` and `:` are payload
    // characters, per the design record's Q1 convention).
    let doc = parse(
        "(subject)character \"Brian\" {\n    (predicate)\"writesBooks\" (object)\"Stewie\" (object)\"Peter\"\n}\n",
    );
    let node = &doc.nodes()[0];
    assert_eq!(annotation_values(node), vec!["subject"]);
    // The child line is one statement node: its name carries the
    // (predicate) annotation, its arguments carry (object).
    let child = &node.children().unwrap().nodes()[0];
    assert_eq!(child.name().value(), "writesBooks");
    assert_eq!(annotation_values(child), vec!["predicate"]);
    assert_eq!(entry_annotation_values(&child.entries()[0]), vec!["object"]);
    assert_eq!(entry_annotation_values(&child.entries()[1]), vec!["object"]);

    let doc = parse(
        "(\"predicate#NotVeryWell\")\"writesBooks\" (\"object#book#author:BrianGriffin\")\"Wish It, Want It, Do It\"\n",
    );
    let node = &doc.nodes()[0];
    assert_eq!(node.name().value(), "writesBooks");
    assert_eq!(annotation_values(node), vec!["predicate#NotVeryWell"]);
    assert_eq!(node.entries().len(), 1);
    assert_eq!(
        entry_annotation_values(&node.entries()[0]),
        vec!["object#book#author:BrianGriffin"]
    );
}

#[test]
fn spec_documents_are_unchanged() {
    // One annotation per part, the spec maximum: lands in the same
    // place the upstream AST kept it, and `ty()` sees it.
    let doc = parse("(ty)node (a)1 key=(b)2 {\n    child\n}\n");
    let node = &doc.nodes()[0];
    assert_eq!(node.ty().map(|t| t.value()), Some("ty"));
    assert_eq!(node.annotations().len(), 1);
    assert_eq!(node.entries()[0].ty().map(|t| t.value()), Some("a"));
    assert_eq!(node.entries()[1].ty().map(|t| t.value()), Some("b"));
    assert!(node.children_annotations().is_empty());
    assert_eq!(doc.to_string(), "(ty)node (a)1 key=(b)2 {\n    child\n}\n");
}

#[cfg(feature = "v1")]
#[test]
fn v1_documents_convert_to_single_annotations() {
    let doc = KdlDocument::parse_v1("(ty)node (a)1 key=(b)2 { child; }\n")
        .expect("v1 document should parse");
    let node = &doc.nodes()[0];
    assert_eq!(annotation_values(node), vec!["ty"]);
    assert_eq!(entry_annotation_values(&node.entries()[0]), vec!["a"]);
    assert_eq!(entry_annotation_values(&node.entries()[1]), vec!["b"]);
    assert!(node.children_annotations().is_empty());
}
