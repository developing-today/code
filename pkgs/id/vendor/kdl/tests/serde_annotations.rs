//! Serde shape of fork annotations (#5753, Packet B1, decision Q3).
//!
//! Annotations are always lists: a node, its children block and each entry
//! expose their annotations as arrays through reserved pseudo-fields, even
//! when there is only one. Reading them never consumes an ordinary entry,
//! and property-key annotations stay out of serde keys.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, PartialEq)]
struct Doc<T> {
    triple: T,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Triple {
    #[serde(rename = "@annotations", default)]
    annotations: Vec<String>,
    #[serde(rename = "#0")]
    subject: String,
    #[serde(rename = "#0@annotations", default)]
    subject_annotations: Vec<String>,
}

#[test]
fn single_node_annotation_is_a_one_element_list() {
    let doc: Doc<Triple> = kdl::de::from_str("triple (object)alice\n").unwrap();
    assert_eq!(
        doc.triple,
        Triple {
            annotations: vec![],
            subject: "alice".into(),
            subject_annotations: vec!["object".into()],
        }
    );
    let doc: Doc<Triple> = kdl::de::from_str("(object)triple alice\n").unwrap();
    assert_eq!(doc.triple.annotations, vec!["object".to_string()]);
}

#[test]
fn multiple_node_annotations_keep_order() {
    let doc: Doc<Triple> =
        kdl::de::from_str("(object)(\"predicate#knows\")triple (person)(agent)alice\n").unwrap();
    assert_eq!(doc.triple.annotations, vec!["object", "predicate#knows"]);
    assert_eq!(doc.triple.subject, "alice");
    assert_eq!(doc.triple.subject_annotations, vec!["person", "agent"]);
}

#[test]
fn annotations_have_array_json_shape() {
    #[derive(Debug, Deserialize)]
    struct Raw {
        #[serde(rename = "@annotations")]
        annotations: serde_json::Value,
        #[serde(rename = "#0")]
        subject: serde_json::Value,
        #[serde(rename = "#0@annotations")]
        subject_annotations: serde_json::Value,
        #[serde(rename = "#@weight@annotations")]
        weight_annotations: serde_json::Value,
    }
    let doc: Doc<Raw> =
        kdl::de::from_str("(object)triple (person)alice weight=(f64)(score)0.5\n").unwrap();
    let raw = doc.triple;
    assert_eq!(raw.annotations, serde_json::json!(["object"]));
    assert_eq!(raw.subject, serde_json::json!("alice"));
    assert_eq!(raw.subject_annotations, serde_json::json!(["person"]));
    assert_eq!(raw.weight_annotations, serde_json::json!(["f64", "score"]));
}

#[test]
fn annotation_fields_do_not_consume_entries() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Rest {
        #[serde(rename = "@annotations")]
        annotations: Vec<String>,
        #[serde(rename = "#0@annotations")]
        first_annotations: Vec<String>,
        #[serde(rename = "#args")]
        args: Vec<String>,
    }
    let doc: Doc<Rest> = kdl::de::from_str("(t)triple (a)x y\n").unwrap();
    assert_eq!(doc.triple.annotations, vec!["t"]);
    assert_eq!(doc.triple.first_annotations, vec!["a"]);
    assert_eq!(doc.triple.args, vec!["x", "y"]);
}

#[test]
fn children_block_annotations() {
    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Block {
        #[serde(rename = "@children-annotations", default)]
        children_annotations: Vec<String>,
        child: String,
    }
    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Top {
        node: Block,
    }
    let top: Top = kdl::de::from_str("node (graph)(named) {\n    child x\n}\n").unwrap();
    assert_eq!(top.node.children_annotations, vec!["graph", "named"]);
    assert_eq!(top.node.child, "x");

    let text = kdl::se::to_string(&top).unwrap();
    assert!(text.contains("(graph)(named)"), "{text}");
    let back: Top = kdl::de::from_str(&text).unwrap();
    assert_eq!(back, top);
}

#[test]
fn property_key_annotations_are_not_serde_keys() {
    #[derive(Debug, Deserialize)]
    struct Props {
        #[serde(rename = "#@weight")]
        weight: f64,
        #[serde(rename = "#@weight@annotations", default)]
        weight_annotations: Vec<String>,
    }
    // `(k)weight` annotates the key; only the value annotation is exposed.
    let doc: Doc<Props> = kdl::de::from_str("triple (k)weight=(v)0.5\n").unwrap();
    assert_eq!(doc.triple.weight, 0.5);
    assert_eq!(doc.triple.weight_annotations, vec!["v"]);

    // Read as a map, the property is keyed by its bare name and nothing else.
    let map: Doc<std::collections::BTreeMap<String, serde_json::Value>> =
        kdl::de::from_str("triple (k)weight=(v)0.5\n").unwrap();
    assert_eq!(map.triple.keys().collect::<Vec<_>>(), vec!["weight"]);
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Top {
    triple: Triple,
}

#[test]
fn round_trip_single_annotation() {
    let value = Top {
        triple: Triple {
            annotations: vec!["object".into()],
            subject: "alice".into(),
            subject_annotations: vec!["person".into()],
        },
    };
    let text = kdl::se::to_string(&value).unwrap();
    assert_eq!(text, "(object)triple (person)alice\n");
    let back: Top = kdl::de::from_str(&text).unwrap();
    assert_eq!(back, value);
}

#[test]
fn round_trip_multiple_annotations() {
    let value = Top {
        triple: Triple {
            annotations: vec!["object".into(), "predicate#knows".into()],
            subject: "alice".into(),
            subject_annotations: vec!["person".into(), "agent".into()],
        },
    };
    let text = kdl::se::to_string(&value).unwrap();
    assert_eq!(
        text,
        "(object)(\"predicate#knows\")triple (person)(agent)alice\n"
    );
    let back: Top = kdl::de::from_str(&text).unwrap();
    assert_eq!(back, value);
}

#[test]
fn entry_annotations_before_their_entry_are_an_error() {
    #[derive(Serialize)]
    struct Backwards {
        #[serde(rename = "#0@annotations")]
        first_annotations: Vec<String>,
        #[serde(rename = "#0")]
        first: String,
    }
    #[derive(Serialize)]
    struct Wrap {
        node: Backwards,
    }
    let err = kdl::se::to_string(&Wrap {
        node: Backwards {
            first_annotations: vec!["a".into()],
            first: "x".into(),
        },
    })
    .unwrap_err();
    assert!(err.to_string().contains("#0@annotations"), "{err}");
}
