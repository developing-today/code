//! Petnames: private names one signed-in account gives another account.
//!
//! A petname lives in its owner's `petnames.jsonl` and nowhere else. It is
//! never replicated, never sent in an envelope and never written to the
//! directory journal. Account names are alleged, so the store refuses a
//! petname that could be taken for someone else's name.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::directory::Refusal;

const MAX_PETNAME_CHARS: usize = 64;
const FINGERPRINT_CHARS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Record {
    Set {
        owner: String,
        account: String,
        name: String,
    },
    Remove {
        owner: String,
        account: String,
    },
}

/// Each owner's petnames, keyed by `(owner, account)`.
#[derive(Debug, Default)]
pub struct Petnames {
    names: BTreeMap<(String, String), String>,
    journal: Option<PathBuf>,
}

impl Petnames {
    /// An empty store that writes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the store at `path`, replaying every line already there.
    ///
    /// # Errors
    ///
    /// Fails if a line is malformed.
    pub fn open(path: &Path) -> Result<Self> {
        let mut store = Self {
            names: BTreeMap::new(),
            journal: Some(path.to_path_buf()),
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(error) => return Err(error.into()),
        };
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record: Record = serde_json::from_str(line)
                .with_context(|| format!("petname journal line {} is malformed", number + 1))?;
            store.apply(record);
        }
        Ok(store)
    }

    /// The petname `owner` gave `account`, if any.
    #[must_use]
    pub fn get(&self, owner: &str, account: &str) -> Option<&str> {
        self.names
            .get(&(owner.to_owned(), account.to_owned()))
            .map(String::as_str)
    }

    /// Every petname `owner` has given, as `(account, name)`.
    pub fn of_owner<'a>(&'a self, owner: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.names
            .iter()
            .filter(move |((o, _), _)| o == owner)
            .map(|((_, account), name)| (account.as_str(), name.as_str()))
    }

    /// Give `account` the petname `name`, replacing any petname it had.
    ///
    /// `alleged` is every account's name as the directory states it, so a
    /// name that looks like a contact's is refused.
    ///
    /// # Errors
    ///
    /// Fails with a [`Refusal::Conflict`] if another contact already has the
    /// name, or if it is confusable with another contact's name. Fails with a
    /// plain error if the name is empty, too long, or has control characters.
    pub fn set<'a>(
        &mut self,
        owner: &str,
        account: &str,
        name: &str,
        alleged: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<()> {
        let name = normalize_name(name)?;
        let folded = fold(&name);
        let shape = skeleton(&name);
        for (other, petname) in self.of_owner(owner) {
            if other == account {
                continue;
            }
            if fold(petname) == folded {
                return Err(Refusal::Conflict(format!(
                    "{petname} is already a petname for another contact"
                ))
                .into());
            }
            if skeleton(petname) == shape {
                return Err(looks_like(petname));
            }
        }
        for (id, alleged_name) in alleged {
            if id == account {
                continue;
            }
            if fold(alleged_name) == folded || skeleton(alleged_name) == shape {
                return Err(looks_like(alleged_name));
            }
        }
        self.commit(Record::Set {
            owner: owner.to_owned(),
            account: account.to_owned(),
            name,
        })
    }

    /// Change the petname `account` already has.
    ///
    /// # Errors
    ///
    /// Fails with a [`Refusal::NotFound`] if `account` has no petname, and as
    /// [`Self::set`] does for the new name.
    pub fn rename<'a>(
        &mut self,
        owner: &str,
        account: &str,
        name: &str,
        alleged: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<()> {
        ensure_has(self, owner, account)?;
        self.set(owner, account, name, alleged)
    }

    /// Forget the petname `owner` gave `account`.
    ///
    /// # Errors
    ///
    /// Fails with a [`Refusal::NotFound`] if there is no such petname.
    pub fn remove(&mut self, owner: &str, account: &str) -> Result<()> {
        ensure_has(self, owner, account)?;
        self.commit(Record::Remove {
            owner: owner.to_owned(),
            account: account.to_owned(),
        })
    }

    fn apply(&mut self, record: Record) {
        match record {
            Record::Set {
                owner,
                account,
                name,
            } => {
                self.names.insert((owner, account), name);
            }
            Record::Remove { owner, account } => {
                self.names.remove(&(owner, account));
            }
        }
    }

    fn commit(&mut self, record: Record) -> Result<()> {
        if let Some(path) = &self.journal {
            let mut line = serde_json::to_vec(&record)?;
            line.push(b'\n');
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            file.write_all(&line)?;
            file.sync_data()?;
        }
        self.apply(record);
        Ok(())
    }
}

fn ensure_has(petnames: &Petnames, owner: &str, account: &str) -> Result<()> {
    if petnames.get(owner, account).is_none() {
        return Err(Refusal::NotFound(format!("no petname for {account}")).into());
    }
    Ok(())
}

fn looks_like(name: &str) -> anyhow::Error {
    Refusal::Conflict(format!("that name looks too much like {name}")).into()
}

/// The name, trimmed, if it is usable as a petname.
///
/// # Errors
///
/// Fails if the name is empty, longer than 64 characters, or has control
/// characters.
pub fn normalize_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "a petname cannot be empty");
    ensure!(
        name.chars().count() <= MAX_PETNAME_CHARS,
        "a petname is at most {MAX_PETNAME_CHARS} characters"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "a petname cannot contain control characters"
    );
    Ok(name.to_owned())
}

/// The name in lower case, for comparing two names as the same spelling.
#[must_use]
pub fn fold(name: &str) -> String {
    name.to_lowercase()
}

/// The name with look-alike characters mapped to one form, so `PayPa1` and
/// `PayPal` compare equal.
#[must_use]
pub fn skeleton(name: &str) -> String {
    let mapped: String = fold(name)
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | '|' => 'l',
            '3' => 'e',
            '5' | '$' => 's',
            '@' => 'a',
            other => other,
        })
        .collect();
    mapped.replace("rn", "m").replace("vv", "w")
}

/// One account as the viewer sees it, for labelling.
#[derive(Debug, Clone, Copy)]
pub struct Contact<'a> {
    /// The account ID, which is its public key in hex.
    pub id: &'a str,
    /// The name the account states for itself.
    pub alleged: &'a str,
    /// The viewer's petname for it, if any.
    pub petname: Option<&'a str>,
}

/// How a contact is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Label {
    /// The viewer's own name for the contact.
    Petname(String),
    /// A petname that another contact shares, with a fingerprint to tell them apart.
    SharedPetname {
        /// The petname.
        name: String,
        /// The first hex characters of the account ID.
        fingerprint: String,
    },
    /// The contact's own, unverified name, with a fingerprint.
    Alleged {
        /// The name the contact states.
        name: String,
        /// The first hex characters of the account ID.
        fingerprint: String,
    },
}

impl Label {
    /// The label as plain text.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Petname(name) => name.clone(),
            Self::SharedPetname { name, fingerprint } => format!("{name} {fingerprint}"),
            Self::Alleged { name, fingerprint } => {
                format!("{name} (unverified {fingerprint})")
            }
        }
    }
}

/// The first hex characters of an account ID, enough to tell two contacts apart.
#[must_use]
pub fn fingerprint(id: &str) -> String {
    id.chars().take(FINGERPRINT_CHARS).collect()
}

/// How each contact is shown: its petname if it has one, otherwise its
/// alleged name marked unverified. Petnames that collide after folding get a
/// fingerprint on both contacts.
#[must_use]
pub fn labels(contacts: &[Contact<'_>]) -> Vec<Label> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for petname in contacts.iter().filter_map(|contact| contact.petname) {
        *counts.entry(fold(petname)).or_default() += 1;
    }
    contacts
        .iter()
        .map(|contact| match contact.petname {
            Some(petname) if counts.get(&fold(petname)).copied().unwrap_or(0) > 1 => {
                Label::SharedPetname {
                    name: petname.to_owned(),
                    fingerprint: fingerprint(contact.id),
                }
            }
            Some(petname) => Label::Petname(petname.to_owned()),
            None => Label::Alleged {
                name: contact.alleged.to_owned(),
                fingerprint: fingerprint(contact.id),
            },
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const ALICE: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";
    const BOB: &str = "bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22";
    const CAROL: &str = "cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33cc33";
    const OWNER: &str = "00000000000000000000000000000000000000000000000000000000000000ff";

    fn names() -> Vec<(&'static str, &'static str)> {
        vec![(ALICE, "Alice"), (BOB, "Bob"), (CAROL, "Carol")]
    }

    fn refusal(error: &anyhow::Error) -> Option<&Refusal> {
        error.downcast_ref::<Refusal>()
    }

    #[test]
    fn a_name_is_refused_for_a_second_contact_either_way() {
        let mut store = Petnames::new();
        store.set(OWNER, ALICE, "Bo", names()).unwrap();
        let taken = store.set(OWNER, BOB, "Bo", names()).unwrap_err();
        assert!(matches!(refusal(&taken), Some(Refusal::Conflict(_))));
        let folded = store.set(OWNER, BOB, "bo", names()).unwrap_err();
        assert!(matches!(refusal(&folded), Some(Refusal::Conflict(_))));
        store.set(OWNER, BOB, "Bobby", names()).unwrap();
        let renamed_onto = store.rename(OWNER, BOB, "BO", names()).unwrap_err();
        assert!(matches!(refusal(&renamed_onto), Some(Refusal::Conflict(_))));
        assert_eq!(store.get(OWNER, BOB), Some("Bobby"));
    }

    #[test]
    fn a_confusable_name_is_refused() {
        let mut store = Petnames::new();
        let alleged = [(ALICE, "PayPal"), (BOB, "Bob")];
        let error = store.set(OWNER, CAROL, "PayPa1", alleged).unwrap_err();
        assert!(matches!(refusal(&error), Some(Refusal::Conflict(_))));
        assert!(store.get(OWNER, CAROL).is_none());
    }

    #[test]
    fn a_confusable_name_already_given_to_a_contact_is_refused() {
        let mut store = Petnames::new();
        store.set(OWNER, ALICE, "Mom", names()).unwrap();
        let error = store.set(OWNER, BOB, "M0m", names()).unwrap_err();
        assert!(matches!(refusal(&error), Some(Refusal::Conflict(_))));
    }

    #[test]
    fn an_unrelated_name_is_accepted() {
        let mut store = Petnames::new();
        store.set(OWNER, CAROL, "Dana", names()).unwrap();
        assert_eq!(store.get(OWNER, CAROL), Some("Dana"));
    }

    #[test]
    fn an_account_may_keep_its_own_alleged_name_as_a_petname() {
        let mut store = Petnames::new();
        store.set(OWNER, ALICE, "Alice", names()).unwrap();
        assert_eq!(store.get(OWNER, ALICE), Some("Alice"));
    }

    #[test]
    fn rename_changes_the_name_and_needs_an_existing_one() {
        let mut store = Petnames::new();
        let missing = store.rename(OWNER, ALICE, "Ali", names()).unwrap_err();
        assert!(matches!(refusal(&missing), Some(Refusal::NotFound(_))));
        store.set(OWNER, ALICE, "Ali", names()).unwrap();
        store.rename(OWNER, ALICE, "Alix", names()).unwrap();
        assert_eq!(store.get(OWNER, ALICE), Some("Alix"));
    }

    #[test]
    fn remove_forgets_the_name_and_frees_it() {
        let mut store = Petnames::new();
        store.set(OWNER, ALICE, "Ali", names()).unwrap();
        store.remove(OWNER, ALICE).unwrap();
        assert!(store.get(OWNER, ALICE).is_none());
        let missing = store.remove(OWNER, ALICE).unwrap_err();
        assert!(matches!(refusal(&missing), Some(Refusal::NotFound(_))));
        store.set(OWNER, BOB, "Ali", names()).unwrap();
    }

    #[test]
    fn petnames_are_private_to_their_owner() {
        let mut store = Petnames::new();
        store.set(OWNER, ALICE, "Ali", names()).unwrap();
        assert!(store.get(BOB, ALICE).is_none());
        store.set(BOB, CAROL, "Ali", names()).unwrap();
    }

    #[test]
    fn restart_reloads_every_change() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("petnames.jsonl");
        {
            let mut store = Petnames::open(&path).unwrap();
            store.set(OWNER, ALICE, "Ali", names()).unwrap();
            store.set(OWNER, BOB, "Bobby", names()).unwrap();
            store.rename(OWNER, BOB, "Robert", names()).unwrap();
            store.remove(OWNER, ALICE).unwrap();
        }
        let store = Petnames::open(&path).unwrap();
        assert!(store.get(OWNER, ALICE).is_none());
        assert_eq!(store.get(OWNER, BOB), Some("Robert"));
    }

    #[test]
    fn a_malformed_line_fails_the_open() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("petnames.jsonl");
        std::fs::write(&path, "not json\n").unwrap();
        assert!(Petnames::open(&path).is_err());
    }

    #[test]
    fn a_petname_is_trimmed_and_bounded() {
        assert_eq!(normalize_name("  Mom  ").unwrap(), "Mom");
        assert!(normalize_name("   ").is_err());
        assert!(normalize_name(&"x".repeat(65)).is_err());
        assert!(normalize_name("a\tb").is_err());
    }

    #[test]
    fn a_petname_is_shown_and_an_alleged_name_is_marked_unverified() {
        let contacts = [
            Contact {
                id: ALICE,
                alleged: "Alice Smith",
                petname: Some("Mom"),
            },
            Contact {
                id: BOB,
                alleged: "Bob",
                petname: None,
            },
        ];
        assert_eq!(
            labels(&contacts),
            vec![
                Label::Petname("Mom".to_owned()),
                Label::Alleged {
                    name: "Bob".to_owned(),
                    fingerprint: "bb22bb".to_owned(),
                },
            ]
        );
        assert_eq!(labels(&contacts)[1].text(), "Bob (unverified bb22bb)");
    }

    #[test]
    fn two_contacts_sharing_a_petname_both_get_a_fingerprint() {
        let contacts = [
            Contact {
                id: ALICE,
                alleged: "Alice",
                petname: Some("Sam"),
            },
            Contact {
                id: BOB,
                alleged: "Bob",
                petname: Some("sam"),
            },
            Contact {
                id: CAROL,
                alleged: "Carol",
                petname: Some("Dana"),
            },
        ];
        let shown = labels(&contacts);
        assert_eq!(
            shown[0],
            Label::SharedPetname {
                name: "Sam".to_owned(),
                fingerprint: "aa11aa".to_owned(),
            }
        );
        assert_eq!(shown[0].text(), "Sam aa11aa");
        assert_eq!(shown[1].text(), "sam bb22bb");
        assert_eq!(shown[2], Label::Petname("Dana".to_owned()));
    }

    #[test]
    fn the_skeleton_maps_look_alikes_together() {
        assert_eq!(skeleton("PayPa1"), skeleton("PayPal"));
        assert_eq!(skeleton("Rnoon"), skeleton("moon"));
        assert_ne!(skeleton("Carol"), skeleton("Bob"));
    }
}
