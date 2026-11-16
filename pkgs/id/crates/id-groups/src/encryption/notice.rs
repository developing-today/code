//! The "scheme: none" notice (P4.10, decision 8).
//!
//! A group created without a key-agreement scheme says so in its metadata and in the status string
//! the UI and the CLI show. The notice is information, not a gate: the group works as it did
//! before. Every group carries the field, so a reader never has to infer the scheme from silence.

use crate::groups::{Policy, Scheme};

/// The metadata key and the status-string prefix for a group's scheme.
pub const SCHEME_KEY: &str = "scheme";

/// The value for a group with no scheme.
pub const NO_SCHEME: &str = "none";

/// The scheme's value in the metadata: `none`, or the scheme's name and version.
pub fn scheme_value(policy: &Policy) -> String {
    match &policy.scheme {
        None => NO_SCHEME.to_owned(),
        Some(Scheme { name, version }) => format!("{name} v{version}"),
    }
}

/// The group's metadata, as key and value pairs. It always has the scheme field.
pub fn metadata(policy: &Policy) -> Vec<(&'static str, String)> {
    vec![(SCHEME_KEY, scheme_value(policy))]
}

/// The status string for the UI and the CLI, such as `scheme: none`.
pub fn status(policy: &Policy) -> String {
    format!("{SCHEME_KEY}: {}", scheme_value(policy))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_a_group_created_without_encryption_carries_the_notice_in_its_metadata() {
        let policy = Policy::default();
        assert_eq!(
            metadata(&policy),
            vec![("scheme", "none".to_owned())],
            "the metadata names the missing scheme"
        );
    }

    #[test]
    fn test_the_status_string_says_scheme_none() {
        assert_eq!(status(&Policy::default()), "scheme: none");
    }

    #[test]
    fn test_a_group_with_a_scheme_names_it_in_the_metadata_and_the_status() {
        let policy = Policy {
            scheme: Some(Scheme {
                name: "data-scheme".to_owned(),
                version: 1,
            }),
            ..Policy::default()
        };
        assert_eq!(
            metadata(&policy),
            vec![("scheme", "data-scheme v1".to_owned())]
        );
        assert_eq!(status(&policy), "scheme: data-scheme v1");
    }

    #[test]
    fn test_the_notice_is_not_a_gate_for_the_group_policy() {
        // The notice describes the policy. A group without a scheme is still valid and can take
        // members, so nothing here may make the policy invalid.
        assert!(Policy::default().is_valid());
    }
}
