//! Serde shape tests for annotation lists (#5753, Packet B).

use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Root {
    item: Item,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Item {
    #[serde(rename = "@annotations")]
    annotations: Vec<String>,
    #[serde(rename = "@children-annotations")]
    children_annotations: Vec<String>,
    #[serde(rename = "#0#type")]
    value_annotations: Vec<String>,
    #[serde(rename = "#@label#type")]
    property_value_annotations: Vec<String>,
    #[serde(rename = "#@label")]
    label: String,
    child: bool,
}

#[test]
fn annotation_lists_are_arrays_and_reserved_fields_round_trip() {
    let input = r#"
        (root-a)(root-b)item (arg-a)(arg-b)42 label=(value-a)(value-b)"text" (children-a)(children-b) {
            child
        }
    "#;

    let actual: Root = kdl::de::from_str(input).expect("annotated KDL should deserialize");
    assert_eq!(
        actual,
        Root {
            item: Item {
                annotations: vec!["root-a".into(), "root-b".into()],
                children_annotations: vec!["children-a".into(), "children-b".into()],
                value_annotations: vec!["arg-a".into(), "arg-b".into()],
                property_value_annotations: vec!["value-a".into(), "value-b".into()],
                label: "text".into(),
                child: true,
            },
        }
    );
}

#[test]
fn single_annotations_still_deserialize_as_one_element_arrays() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Annotated {
        #[serde(rename = "@annotations")]
        annotations: Vec<String>,
        #[serde(rename = "#0#type")]
        value_annotations: Vec<String>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Root {
        root: Annotated,
    }

    let actual: Root = kdl::de::from_str("(node)root (value)1").unwrap();
    assert_eq!(actual.root.annotations, ["node"]);
    assert_eq!(actual.root.value_annotations, ["value"]);
}

#[test]
fn property_key_annotations_are_not_created_as_serde_map_keys() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Props {
        value: String,
    }
    #[derive(Debug, Deserialize, PartialEq)]
    struct Root {
        root: Props,
    }

    let actual: Root = kdl::de::from_str("root (key-annotation)value=ok").unwrap();
    assert_eq!(actual.root, Props { value: "ok".into() });
}
