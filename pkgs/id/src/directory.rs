//! Accounts, groups, memberships and friends for one server.
//!
//! The directory is a pure model. Every change is a [`DirectoryEntry`]; the
//! same `apply` path runs live and on replay, and each entry is applied to a
//! copy first, so a refused change leaves no trace and a bad journal is
//! refused rather than partly applied. Account secrets never enter an entry,
//! only their SHA-256 digest does.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;

use crate::world::{WorldScopes, hex_encode};

const KEY_DOMAIN: &[u8] = b"id-account-v1";
const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 512;

/// What an actor may do inside a group, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Sees the group and its own membership.
    Access,
    /// Sees members, their levels, and the permission set.
    Read,
    /// Edits the group's description.
    Write,
    /// Adds and removes members at or below write.
    Manage,
    /// Everything, including the permission set, name and deletion.
    Admin,
}

/// The default permission set an account gets from its verification state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Capabilities that name no account.
    Anonymous,
    /// Signed-up accounts that are not verified.
    Registered,
    /// Accounts whose email or identity was confirmed.
    Verified,
}

/// A member of a group: an account, or a nested group.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Member {
    /// An account, by its public-key ID.
    Account {
        /// The account ID.
        id: String,
    },
    /// A nested group, by its ID.
    Group {
        /// The group ID.
        id: u64,
    },
}

/// A signed-up account as the directory knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// The account ID, a hex Ed25519 public key.
    pub id: String,
    /// The display name.
    pub name: String,
    /// Whether the server has confirmed the account's email or identity.
    pub verified: bool,
}

/// A group and the permission set its members hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// The group ID.
    pub id: u64,
    /// The group name.
    pub name: String,
    /// The group description.
    pub description: String,
    /// The permissions members gain from this group.
    pub scopes: WorldScopes,
}

/// One durable change to the directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum DirectoryEntry {
    /// A new account; only the digest of its secret is stored.
    AccountSignedUp {
        /// The account ID.
        id: String,
        /// The display name.
        name: String,
        /// SHA-256 of the account secret.
        digest: String,
    },
    /// The server confirmed an account.
    AccountVerified {
        /// The account ID.
        id: String,
    },
    /// A group was created with its founder as admin.
    GroupCreated {
        /// The group ID.
        id: u64,
        /// The group name.
        name: String,
        /// The group description.
        description: String,
        /// The permission bits.
        scopes: u8,
        /// The account that founded the group.
        founder: String,
    },
    /// A group's name, description or permission set changed.
    GroupUpdated {
        /// The group ID.
        id: u64,
        /// The group name.
        name: String,
        /// The group description.
        description: String,
        /// The permission bits.
        scopes: u8,
    },
    /// A group and its memberships were removed.
    GroupDeleted {
        /// The group ID.
        id: u64,
    },
    /// A member gained, changed or lost a level in a group.
    MemberSet {
        /// The group ID.
        group: u64,
        /// The member.
        member: Member,
        /// The new level, or `None` to remove the member.
        level: Option<Level>,
    },
    /// A tier's permission set changed.
    TierSet {
        /// The tier.
        tier: Tier,
        /// The permission bits.
        scopes: u8,
    },
    /// A friend request was made from one account to another.
    FriendRequested {
        /// The requesting account.
        from: String,
        /// The account asked.
        to: String,
    },
    /// A pending request was accepted.
    FriendAccepted {
        /// The account that made the request.
        from: String,
        /// The account that accepted it.
        to: String,
    },
    /// A friendship ended.
    FriendRemoved {
        /// One side of the friendship.
        a: String,
        /// The other side.
        b: String,
    },
}

/// Who is making a change: the server itself, or a signed-in account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor<'a> {
    /// The server, trusted to act for any account.
    Server,
    /// A signed-in account.
    Account(&'a str),
}

/// What a friend envelope says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    /// A request to become friends.
    FriendRequest,
    /// Acceptance of a request.
    FriendAccept,
}

/// A friend request or acceptance, signed by its sender's account key. Anyone
/// holding it can check the signature without asking the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// What the envelope says.
    pub kind: EnvelopeKind,
    /// The sender's account ID.
    pub from: String,
    /// The recipient's account ID.
    pub to: String,
    /// When it was made, in Unix milliseconds.
    pub at: u64,
    /// Hex Ed25519 signature over the canonical body.
    pub signature: String,
}

#[derive(Serialize)]
struct SignedBody<'a> {
    kind: EnvelopeKind,
    from: &'a str,
    to: &'a str,
    at: u64,
}

impl Envelope {
    fn signed(key: &SigningKey, kind: EnvelopeKind, from: &str, to: &str, at: u64) -> Result<Self> {
        let body = serde_json::to_vec(&SignedBody { kind, from, to, at })?;
        let signature = key.sign(&body);
        Ok(Self {
            kind,
            from: from.to_owned(),
            to: to.to_owned(),
            at,
            signature: hex_encode(&signature.to_bytes()),
        })
    }

    /// Check the signature against the sender's account key.
    ///
    /// # Errors
    ///
    /// Fails if the sender is not a key, the signature is malformed, or it
    /// does not match the body.
    pub fn verify(&self) -> Result<()> {
        let key = VerifyingKey::from_bytes(
            &hex_to_array::<32>(&self.from).context("malformed envelope sender")?,
        )
        .context("envelope sender is not a key")?;
        let signature = Signature::from_bytes(
            &hex_to_array::<64>(&self.signature).context("malformed envelope signature")?,
        );
        let body = serde_json::to_vec(&SignedBody {
            kind: self.kind,
            from: &self.from,
            to: &self.to,
            at: self.at,
        })?;
        key.verify(&body, &signature)
            .context("envelope signature does not verify")
    }
}

/// The directory of accounts, groups, memberships and friends on one server.
#[derive(Clone, Debug)]
pub struct Directory {
    accounts: BTreeMap<String, Account>,
    digests: BTreeMap<String, [u8; 32]>,
    groups: BTreeMap<u64, Group>,
    members: BTreeMap<(u64, Member), Level>,
    tiers: BTreeMap<Tier, WorldScopes>,
    friends: BTreeSet<(String, String)>,
    requests: BTreeSet<(String, String)>,
    next_group: u64,
    journal: Option<PathBuf>,
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}

impl Directory {
    /// An empty directory with the default tier permission sets.
    #[must_use]
    pub fn new() -> Self {
        let tiers = BTreeMap::from([
            (Tier::Anonymous, WorldScopes::GUEST),
            (Tier::Registered, WorldScopes::GUEST),
            (
                Tier::Verified,
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
            ),
        ]);
        Self {
            accounts: BTreeMap::new(),
            digests: BTreeMap::new(),
            groups: BTreeMap::new(),
            members: BTreeMap::new(),
            tiers,
            friends: BTreeSet::new(),
            requests: BTreeSet::new(),
            next_group: 1,
            journal: None,
        }
    }

    /// Rebuild the directory from its journal and journal every later change
    /// to it. A change is journaled before it becomes visible.
    ///
    /// # Errors
    ///
    /// Fails if the journal cannot be replayed.
    pub fn open(path: &Path) -> Result<Self> {
        let mut directory = Self::replay(path)?;
        directory.journal = Some(path.to_path_buf());
        Ok(directory)
    }

    /// Rebuild the directory from its journal, one line per entry.
    ///
    /// # Errors
    ///
    /// Fails if a line is malformed or an entry breaks a directory rule.
    pub fn replay(path: &Path) -> Result<Self> {
        let mut directory = Self::new();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(directory),
            Err(error) => return Err(error.into()),
        };
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: DirectoryEntry = serde_json::from_str(line)
                .with_context(|| format!("directory journal line {} is malformed", number + 1))?;
            directory
                .apply(&entry)
                .with_context(|| format!("directory journal line {} is refused", number + 1))?;
        }
        Ok(directory)
    }

    /// Durably append one entry to the journal.
    ///
    /// # Errors
    ///
    /// Fails if the journal cannot be written or synced.
    pub fn append(path: &Path, entry: &DirectoryEntry) -> Result<()> {
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(&line)?;
        file.sync_data()?;
        Ok(())
    }

    /// Create an account. The returned credential is shown once; the server
    /// keeps only its digest.
    ///
    /// # Errors
    ///
    /// Fails if the name is empty, too long, or contains control characters.
    pub fn sign_up(&mut self, name: &str) -> Result<(String, DirectoryEntry)> {
        let mut secret = [0_u8; 32];
        rand::rng().fill(&mut secret);
        let id = hex_encode(&signing_key(&secret).verifying_key().to_bytes());
        let entry = DirectoryEntry::AccountSignedUp {
            id: id.clone(),
            name: name.to_owned(),
            digest: hex_encode(&Sha256::digest(secret)),
        };
        self.apply(&entry)?;
        Ok((format!("acct.{id}.{}", hex_encode(&secret)), entry))
    }

    /// The account a credential belongs to, if it is valid.
    #[must_use]
    pub fn account_for(&self, credential: &str) -> Option<String> {
        self.authenticate(credential).ok().map(|(id, _)| id)
    }

    /// Mark an account as verified. Only the server may do this.
    ///
    /// # Errors
    ///
    /// Fails for account actors and unknown accounts.
    pub fn verify_account(&mut self, actor: Actor<'_>, id: &str) -> Result<DirectoryEntry> {
        ensure!(actor == Actor::Server, "only the server verifies accounts");
        let entry = DirectoryEntry::AccountVerified { id: id.to_owned() };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Set the permission set a tier carries. Only the server may do this.
    ///
    /// # Errors
    ///
    /// Fails for account actors and unknown permission bits.
    pub fn set_tier(
        &mut self,
        actor: Actor<'_>,
        tier: Tier,
        scopes: WorldScopes,
    ) -> Result<DirectoryEntry> {
        ensure!(
            actor == Actor::Server,
            "only the server sets tier permissions"
        );
        let entry = DirectoryEntry::TierSet {
            tier,
            scopes: scopes.bits(),
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Create a group; the founder becomes its admin.
    ///
    /// # Errors
    ///
    /// Fails if the founder is not an account or the name is invalid.
    pub fn create_group(
        &mut self,
        founder: &str,
        name: &str,
        description: &str,
        scopes: WorldScopes,
    ) -> Result<(u64, DirectoryEntry)> {
        let id = self.next_group;
        let entry = DirectoryEntry::GroupCreated {
            id,
            name: name.to_owned(),
            description: description.to_owned(),
            scopes: scopes.bits(),
            founder: founder.to_owned(),
        };
        self.apply(&entry)?;
        Ok((id, entry))
    }

    /// Change a group. A description needs `write`; a name or permission set
    /// needs `admin`.
    ///
    /// # Errors
    ///
    /// Fails when the actor lacks the level, or the change breaks a rule.
    pub fn update_group(
        &mut self,
        actor: Actor<'_>,
        group: u64,
        name: &str,
        description: &str,
        scopes: WorldScopes,
    ) -> Result<DirectoryEntry> {
        let name = clean_name(name)?;
        let description = clean_description(description)?;
        let current = self.groups.get(&group).context("no such group")?;
        let structural = name != current.name || scopes != current.scopes;
        let needed = if structural {
            Level::Admin
        } else {
            Level::Write
        };
        ensure!(
            self.allowed(actor, group, needed),
            "not allowed to change this group"
        );
        let entry = DirectoryEntry::GroupUpdated {
            id: group,
            name,
            description,
            scopes: scopes.bits(),
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Delete a group and every membership it holds or is held by.
    ///
    /// # Errors
    ///
    /// Fails unless the actor is an admin, or if deleting would leave a group
    /// without an admin.
    pub fn delete_group(&mut self, actor: Actor<'_>, group: u64) -> Result<DirectoryEntry> {
        ensure!(
            self.allowed(actor, group, Level::Admin),
            "only an admin deletes a group"
        );
        let entry = DirectoryEntry::GroupDeleted { id: group };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Give a member a level in a group, or remove it with `None`.
    ///
    /// Managers may add and change members up to `write`, and may not touch
    /// their own level or any member at `manage` or above. Only admins set
    /// `manage` or `admin`.
    ///
    /// # Errors
    ///
    /// Fails when the actor lacks the level, or when the change would leave a
    /// group without an admin or create a nesting cycle.
    pub fn set_member(
        &mut self,
        actor: Actor<'_>,
        group: u64,
        member: Member,
        level: Option<Level>,
    ) -> Result<DirectoryEntry> {
        ensure!(
            self.allowed(actor, group, Level::Manage),
            "not allowed to manage this group"
        );
        if let Actor::Account(me) = actor
            && self.level_in(me, group) != Some(Level::Admin)
        {
            ensure!(
                member != (Member::Account { id: me.to_owned() }),
                "you cannot change your own level"
            );
            ensure!(
                level.is_none_or(|level| level <= Level::Write),
                "managers can grant up to write"
            );
            let current = self.members.get(&(group, member.clone())).copied();
            ensure!(
                current.is_none_or(|current| current < Level::Manage),
                "managers cannot change managers or admins"
            );
        }
        let entry = DirectoryEntry::MemberSet {
            group,
            member,
            level,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// The level an account holds in a group, directly or through nested
    /// groups. A nested group passes on at most the level it is held at.
    #[must_use]
    pub fn level_in(&self, account: &str, group: u64) -> Option<Level> {
        let direct = self
            .members
            .get(&(
                group,
                Member::Account {
                    id: account.to_owned(),
                },
            ))
            .copied();
        self.members
            .range((group, Member::Group { id: 0 })..=(group, Member::Group { id: u64::MAX }))
            .fold(direct, |best, ((_, member), held_at)| match member {
                Member::Group { id } => match self.level_in(account, *id) {
                    Some(inner) => best.max(Some(inner.min(*held_at))),
                    None => best,
                },
                Member::Account { .. } => best,
            })
    }

    /// Every permission an account may use: its tier's set, plus the set of
    /// each group it belongs to. A capability's own scopes still narrow this.
    #[must_use]
    pub fn ceiling(&self, account: &str) -> WorldScopes {
        let tier = match self.accounts.get(account) {
            Some(found) if found.verified => Tier::Verified,
            Some(_) => Tier::Registered,
            None => Tier::Anonymous,
        };
        self.groups
            .values()
            .fold(self.tiers[&tier], |scopes, group| {
                if self.level_in(account, group.id).is_some() {
                    scopes.union(group.scopes)
                } else {
                    scopes
                }
            })
    }

    /// The permission set an anonymous capability is held to.
    #[must_use]
    pub fn anonymous_ceiling(&self) -> WorldScopes {
        self.tiers[&Tier::Anonymous]
    }

    /// The group with this ID, if it exists.
    #[must_use]
    pub fn group(&self, id: u64) -> Option<&Group> {
        self.groups.get(&id)
    }

    /// The account with this ID, if it exists.
    #[must_use]
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.get(id)
    }

    /// The direct members of a group with their levels, in order.
    #[must_use]
    pub fn members_of(&self, group: u64) -> Vec<(Member, Level)> {
        self.members
            .range(
                (group, Member::Account { id: String::new() })
                    ..=(group, Member::Group { id: u64::MAX }),
            )
            .map(|((_, member), level)| (member.clone(), *level))
            .collect()
    }

    /// Friends of an account, in order.
    #[must_use]
    pub fn friends_of(&self, id: &str) -> Vec<String> {
        self.friends
            .iter()
            .filter_map(|(a, b)| {
                if a == id {
                    Some(b.clone())
                } else if b == id {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Ask `to` to be friends, signed with the requester's account key.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential, a request to oneself, an existing
    /// friendship, or a pending request in either direction.
    pub fn request_friend(
        &mut self,
        credential: &str,
        to: &str,
        at: u64,
    ) -> Result<(Envelope, DirectoryEntry)> {
        let (from, key) = self.authenticate(credential)?;
        let envelope = Envelope::signed(&key, EnvelopeKind::FriendRequest, &from, to, at)?;
        let entry = DirectoryEntry::FriendRequested {
            from,
            to: to.to_owned(),
        };
        self.apply(&entry)?;
        Ok((envelope, entry))
    }

    /// Accept a pending request from `from`, returning the signed acceptance.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential or when no request is pending.
    pub fn accept_friend(
        &mut self,
        credential: &str,
        from: &str,
        at: u64,
    ) -> Result<(Envelope, DirectoryEntry)> {
        let (me, key) = self.authenticate(credential)?;
        let envelope = Envelope::signed(&key, EnvelopeKind::FriendAccept, &me, from, at)?;
        let entry = DirectoryEntry::FriendAccepted {
            from: from.to_owned(),
            to: me,
        };
        self.apply(&entry)?;
        Ok((envelope, entry))
    }

    /// Record a request another server signed, addressed to a local account.
    ///
    /// # Errors
    ///
    /// Fails if the signature does not verify, the envelope is not a request,
    /// or it is addressed to no local account.
    pub fn receive_request(&mut self, envelope: &Envelope) -> Result<DirectoryEntry> {
        envelope.verify()?;
        ensure!(
            envelope.kind == EnvelopeKind::FriendRequest,
            "envelope is not a friend request"
        );
        ensure!(
            self.accounts.contains_key(&envelope.to),
            "request is not for an account on this server"
        );
        let entry = DirectoryEntry::FriendRequested {
            from: envelope.from.clone(),
            to: envelope.to.clone(),
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Record a signed acceptance of one of this server's requests.
    ///
    /// # Errors
    ///
    /// Fails if the signature does not verify, the envelope is not an
    /// acceptance, or no matching request is pending.
    pub fn receive_accept(&mut self, envelope: &Envelope) -> Result<DirectoryEntry> {
        envelope.verify()?;
        ensure!(
            envelope.kind == EnvelopeKind::FriendAccept,
            "envelope is not a friend acceptance"
        );
        ensure!(
            self.accounts.contains_key(&envelope.to),
            "acceptance is not for an account on this server"
        );
        let entry = DirectoryEntry::FriendAccepted {
            from: envelope.to.clone(),
            to: envelope.from.clone(),
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// End a friendship.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential or when the two are not friends.
    pub fn remove_friend(&mut self, credential: &str, other: &str) -> Result<DirectoryEntry> {
        let (me, _) = self.authenticate(credential)?;
        let entry = DirectoryEntry::FriendRemoved {
            a: me,
            b: other.to_owned(),
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    fn authenticate(&self, credential: &str) -> Result<(String, SigningKey)> {
        let refused = || anyhow::anyhow!("invalid account credential");
        let mut parts = credential.split('.');
        let (Some("acct"), Some(id), Some(secret), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(refused());
        };
        let secret = hex_to_array::<32>(secret).ok_or_else(refused)?;
        let stored = self.digests.get(id).ok_or_else(refused)?;
        let digest: [u8; 32] = Sha256::digest(secret).into();
        if !bool::from(digest.ct_eq(stored)) {
            return Err(refused());
        }
        let key = signing_key(&secret);
        ensure!(
            hex_encode(&key.verifying_key().to_bytes()) == id,
            "account key does not match its ID"
        );
        Ok((id.to_owned(), key))
    }

    fn allowed(&self, actor: Actor<'_>, group: u64, needed: Level) -> bool {
        match actor {
            Actor::Server => true,
            Actor::Account(id) => self
                .level_in(id, group)
                .is_some_and(|level| level >= needed),
        }
    }

    fn apply(&mut self, entry: &DirectoryEntry) -> Result<()> {
        let mut next = self.clone();
        next.change(entry)?;
        next.ensure_admins()?;
        if let Some(path) = &self.journal {
            Self::append(path, entry)?;
        }
        *self = next;
        Ok(())
    }

    fn change(&mut self, entry: &DirectoryEntry) -> Result<()> {
        match entry {
            DirectoryEntry::AccountSignedUp { id, name, digest } => {
                ensure_account_id(id)?;
                ensure!(!self.accounts.contains_key(id), "account already exists");
                let digest = hex_to_array::<32>(digest).context("malformed account digest")?;
                self.accounts.insert(
                    id.clone(),
                    Account {
                        id: id.clone(),
                        name: clean_name(name)?,
                        verified: false,
                    },
                );
                self.digests.insert(id.clone(), digest);
            }
            DirectoryEntry::AccountVerified { id } => {
                self.accounts
                    .get_mut(id)
                    .context("no such account")?
                    .verified = true;
            }
            DirectoryEntry::GroupCreated {
                id,
                name,
                description,
                scopes,
                founder,
            } => {
                ensure!(*id == self.next_group, "group {id} is out of sequence");
                ensure!(
                    self.accounts.contains_key(founder),
                    "founder is not an account"
                );
                self.groups.insert(
                    *id,
                    Group {
                        id: *id,
                        name: clean_name(name)?,
                        description: clean_description(description)?,
                        scopes: permissions(*scopes)?,
                    },
                );
                self.members.insert(
                    (
                        *id,
                        Member::Account {
                            id: founder.clone(),
                        },
                    ),
                    Level::Admin,
                );
                self.next_group = id.checked_add(1).context("group ID exhausted")?;
            }
            DirectoryEntry::GroupUpdated {
                id,
                name,
                description,
                scopes,
            } => {
                let name = clean_name(name)?;
                let description = clean_description(description)?;
                let scopes = permissions(*scopes)?;
                let group = self.groups.get_mut(id).context("no such group")?;
                group.name = name;
                group.description = description;
                group.scopes = scopes;
            }
            DirectoryEntry::GroupDeleted { id } => {
                ensure!(self.groups.remove(id).is_some(), "no such group");
                self.members.retain(|(container, member), _| {
                    container != id && *member != (Member::Group { id: *id })
                });
            }
            DirectoryEntry::MemberSet {
                group,
                member,
                level,
            } => {
                ensure!(self.groups.contains_key(group), "no such group");
                match member {
                    Member::Account { id } => {
                        ensure!(self.accounts.contains_key(id), "no such account");
                    }
                    Member::Group { id } => {
                        ensure!(self.groups.contains_key(id), "no such group");
                        ensure!(!self.reaches(*id, *group), "nesting would create a cycle");
                    }
                }
                match level {
                    Some(level) => {
                        self.members.insert((*group, member.clone()), *level);
                    }
                    None => {
                        self.members.remove(&(*group, member.clone()));
                    }
                }
            }
            DirectoryEntry::TierSet { tier, scopes } => {
                self.tiers.insert(*tier, permissions(*scopes)?);
            }
            DirectoryEntry::FriendRequested { from, to } => {
                ensure_account_id(from)?;
                ensure_account_id(to)?;
                ensure!(from != to, "no one befriends themselves");
                ensure!(!self.are_friends(from, to), "already friends");
                ensure!(
                    !self.requests.contains(&(from.clone(), to.clone()))
                        && !self.requests.contains(&(to.clone(), from.clone())),
                    "a request is already pending"
                );
                self.requests.insert((from.clone(), to.clone()));
            }
            DirectoryEntry::FriendAccepted { from, to } => {
                ensure!(
                    self.requests.remove(&(from.clone(), to.clone())),
                    "no pending request from {from} to {to}"
                );
                self.friends.insert(pair(from, to));
            }
            DirectoryEntry::FriendRemoved { a, b } => {
                ensure!(self.friends.remove(&pair(a, b)), "not friends");
            }
        }
        Ok(())
    }

    /// Whether two accounts are friends on this server.
    #[must_use]
    pub fn are_friends(&self, a: &str, b: &str) -> bool {
        self.friends.contains(&pair(a, b))
    }

    /// Whether `target` is `from` or is nested, at any depth, inside it.
    fn reaches(&self, from: u64, target: u64) -> bool {
        if from == target {
            return true;
        }
        self.members
            .range((from, Member::Group { id: 0 })..=(from, Member::Group { id: u64::MAX }))
            .any(|((_, member), _)| match member {
                Member::Group { id } => self.reaches(*id, target),
                Member::Account { .. } => false,
            })
    }

    fn ensure_admins(&self) -> Result<()> {
        for group in self.groups.keys() {
            let has_admin = self
                .accounts
                .keys()
                .any(|account| self.level_in(account, *group) == Some(Level::Admin));
            ensure!(has_admin, "group {group} would have no admin");
        }
        Ok(())
    }
}

fn signing_key(secret: &[u8; 32]) -> SigningKey {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update(secret);
    SigningKey::from_bytes(&hasher.finalize().into())
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

fn ensure_account_id(id: &str) -> Result<()> {
    ensure!(
        hex_to_array::<32>(id).is_some(),
        "account ID must be 64 hex characters"
    );
    Ok(())
}

fn permissions(bits: u8) -> Result<WorldScopes> {
    WorldScopes::from_bits(bits).context("unknown permission bits")
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "name must not be empty");
    ensure!(
        name.chars().count() <= MAX_NAME_CHARS,
        "name must be at most {MAX_NAME_CHARS} characters"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "name must not contain control characters"
    );
    Ok(name.to_owned())
}

fn clean_description(description: &str) -> Result<String> {
    let description = description.trim();
    ensure!(
        description.chars().count() <= MAX_DESCRIPTION_CHARS,
        "description must be at most {MAX_DESCRIPTION_CHARS} characters"
    );
    ensure!(
        !description.chars().any(|c| c.is_control() && c != '\n'),
        "description must not contain control characters"
    );
    Ok(description.to_owned())
}

fn hex_to_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0_u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn all() -> WorldScopes {
        WorldScopes::GUEST.union(WorldScopes::DELEGATE)
    }

    fn account(directory: &mut Directory, name: &str) -> (String, String) {
        let (credential, _) = directory.sign_up(name).unwrap();
        let id = directory.account_for(&credential).unwrap();
        (id, credential)
    }

    #[test]
    fn a_credential_authenticates_only_its_own_account() {
        let mut directory = Directory::new();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        assert_eq!(directory.account_for(&ann_credential), Some(ann.clone()));
        assert_ne!(ann, bo);
        let forged = format!("acct.{ann}.{}", "00".repeat(32));
        assert_eq!(directory.account_for(&forged), None);
        assert_eq!(directory.account_for("acct.nope"), None);
    }

    #[test]
    fn the_journal_never_holds_a_credential_secret() {
        let mut directory = Directory::new();
        let (credential, entry) = directory.sign_up("Ann").unwrap();
        let secret = credential.rsplit('.').next().unwrap();
        let text = serde_json::to_string(&entry).unwrap();
        assert!(!text.contains(secret));
    }

    #[test]
    fn a_creator_is_the_only_admin_and_can_add_managers() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (group, _) = directory.create_group(&ann, "Crew", "", all()).unwrap();
        assert_eq!(directory.level_in(&ann, group), Some(Level::Admin));
        directory
            .set_member(
                Actor::Account(&ann),
                group,
                Member::Account { id: bo.clone() },
                Some(Level::Manage),
            )
            .unwrap();
        assert_eq!(directory.level_in(&bo, group), Some(Level::Manage));
    }

    #[test]
    fn managers_add_up_to_write_and_never_touch_managers_or_admins() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (cy, _) = account(&mut directory, "Cy");
        let (di, _) = account(&mut directory, "Di");
        let (group, _) = directory.create_group(&ann, "Crew", "", all()).unwrap();
        let manage = Some(Level::Manage);
        directory
            .set_member(
                Actor::Account(&ann),
                group,
                Member::Account { id: bo.clone() },
                manage,
            )
            .unwrap();
        let cy_member = Member::Account { id: cy.clone() };
        assert!(
            directory
                .set_member(
                    Actor::Account(&bo),
                    group,
                    cy_member.clone(),
                    Some(Level::Admin)
                )
                .is_err(),
            "managers cannot grant admin"
        );
        assert!(
            directory
                .set_member(
                    Actor::Account(&bo),
                    group,
                    cy_member.clone(),
                    Some(Level::Manage)
                )
                .is_err(),
            "managers cannot grant manage"
        );
        directory
            .set_member(
                Actor::Account(&bo),
                group,
                cy_member.clone(),
                Some(Level::Write),
            )
            .unwrap();
        assert!(
            directory
                .set_member(
                    Actor::Account(&bo),
                    group,
                    Member::Account { id: ann.clone() },
                    None
                )
                .is_err(),
            "managers cannot remove admins"
        );
        assert!(
            directory
                .set_member(
                    Actor::Account(&bo),
                    group,
                    Member::Account { id: bo.clone() },
                    None
                )
                .is_err(),
            "managers cannot change their own level"
        );
        let di_member = Member::Account { id: di.clone() };
        assert!(
            directory
                .set_member(Actor::Account(&cy), group, di_member, Some(Level::Read))
                .is_err(),
            "write cannot manage"
        );
    }

    #[test]
    fn write_edits_the_description_and_only_admins_rename_or_rescope() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (group, _) = directory.create_group(&ann, "Crew", "", all()).unwrap();
        directory
            .set_member(
                Actor::Account(&ann),
                group,
                Member::Account { id: bo.clone() },
                Some(Level::Write),
            )
            .unwrap();
        directory
            .update_group(Actor::Account(&bo), group, "Crew", "We meet Fridays", all())
            .unwrap();
        assert!(
            directory
                .update_group(Actor::Account(&bo), group, "Renamed", "", all())
                .is_err()
        );
        assert!(
            directory
                .update_group(Actor::Account(&bo), group, "Crew", "", WorldScopes::GUEST)
                .is_err()
        );
        assert!(directory.delete_group(Actor::Account(&bo), group).is_err());
    }

    #[test]
    fn a_group_always_keeps_an_admin() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (group, _) = directory.create_group(&ann, "Crew", "", all()).unwrap();
        assert!(
            directory
                .set_member(
                    Actor::Server,
                    group,
                    Member::Account { id: ann.clone() },
                    Some(Level::Read)
                )
                .is_err(),
            "the last admin cannot be demoted"
        );
        assert!(
            directory
                .set_member(
                    Actor::Server,
                    group,
                    Member::Account { id: ann.clone() },
                    None
                )
                .is_err(),
            "the last admin cannot be removed"
        );
    }

    #[test]
    fn nesting_passes_the_lower_level_and_refuses_cycles() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (inner, _) = directory.create_group(&ann, "Inner", "", all()).unwrap();
        let (outer, _) = directory.create_group(&ann, "Outer", "", all()).unwrap();
        directory
            .set_member(
                Actor::Server,
                inner,
                Member::Account { id: bo.clone() },
                Some(Level::Manage),
            )
            .unwrap();
        directory
            .set_member(
                Actor::Server,
                outer,
                Member::Group { id: inner },
                Some(Level::Read),
            )
            .unwrap();
        assert_eq!(
            directory.level_in(&bo, outer),
            Some(Level::Read),
            "a nested member gets at most the level its group holds"
        );
        assert!(
            directory
                .set_member(
                    Actor::Server,
                    inner,
                    Member::Group { id: outer },
                    Some(Level::Read)
                )
                .is_err(),
            "a cycle is refused"
        );
        assert!(
            directory
                .set_member(
                    Actor::Server,
                    inner,
                    Member::Group { id: inner },
                    Some(Level::Read)
                )
                .is_err(),
            "a group may not contain itself"
        );
    }

    #[test]
    fn an_account_may_belong_to_several_groups_and_ceilings_union() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (first, _) = directory
            .create_group(&ann, "First", "", WorldScopes::GUEST)
            .unwrap();
        let (second, _) = directory
            .create_group(&ann, "Second", "", WorldScopes::JOIN)
            .unwrap();
        directory
            .set_member(
                Actor::Server,
                first,
                Member::Account { id: bo.clone() },
                Some(Level::Read),
            )
            .unwrap();
        directory
            .set_member(
                Actor::Server,
                second,
                Member::Account { id: bo.clone() },
                Some(Level::Access),
            )
            .unwrap();
        assert_eq!(directory.level_in(&bo, first), Some(Level::Read));
        assert_eq!(directory.level_in(&bo, second), Some(Level::Access));
        assert_eq!(directory.ceiling(&bo), WorldScopes::GUEST);
        let (third, _) = directory
            .create_group(&ann, "Third", "", WorldScopes::DELEGATE)
            .unwrap();
        assert_eq!(
            directory.ceiling(&bo),
            WorldScopes::GUEST,
            "not a member yet"
        );
        directory
            .set_member(
                Actor::Server,
                third,
                Member::Account { id: bo.clone() },
                Some(Level::Access),
            )
            .unwrap();
        assert_eq!(directory.ceiling(&bo), all());
    }

    #[test]
    fn tiers_set_the_ceiling_and_verification_raises_it() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        assert_eq!(directory.ceiling(&ann), WorldScopes::GUEST);
        assert!(
            directory
                .set_tier(Actor::Account(&ann), Tier::Verified, all())
                .is_err()
        );
        directory.verify_account(Actor::Server, &ann).unwrap();
        assert_eq!(
            directory.ceiling(&ann),
            WorldScopes::GUEST.union(WorldScopes::DELEGATE)
        );
        directory
            .set_tier(Actor::Server, Tier::Registered, WorldScopes::JOIN)
            .unwrap();
        let (bo, _) = account(&mut directory, "Bo");
        assert_eq!(directory.ceiling(&bo), WorldScopes::JOIN);
        assert_eq!(directory.anonymous_ceiling(), WorldScopes::GUEST);
    }

    #[test]
    fn friends_connect_by_signed_envelopes_on_one_server() {
        let mut directory = Directory::new();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, bo_credential) = account(&mut directory, "Bo");
        let (request, _) = directory.request_friend(&ann_credential, &bo, 1).unwrap();
        assert!(request.verify().is_ok());
        assert!(
            directory.request_friend(&bo_credential, &ann, 2).is_err(),
            "a request is already pending"
        );
        let (accept, _) = directory.accept_friend(&bo_credential, &ann, 3).unwrap();
        assert!(accept.verify().is_ok());
        assert_eq!(directory.friends_of(&ann), vec![bo.clone()]);
        assert_eq!(directory.friends_of(&bo), vec![ann.clone()]);
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(directory.friends_of(&ann).is_empty());
    }

    #[test]
    fn a_tampered_envelope_does_not_verify() {
        let mut directory = Directory::new();
        let (_, ann_credential) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (mut request, _) = directory.request_friend(&ann_credential, &bo, 1).unwrap();
        request.at = 2;
        assert!(request.verify().is_err());
    }

    #[test]
    fn friendship_crosses_servers_with_envelopes_alone() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home.request_friend(&ann_credential, &bo, 1).unwrap();
        away.receive_request(&request).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, 2).unwrap();
        home.receive_accept(&accept).unwrap();
        assert_eq!(home.friends_of(&ann), vec![bo.clone()]);
        assert_eq!(away.friends_of(&bo), vec![ann.clone()]);
        assert!(
            away.receive_request(&accept).is_err(),
            "an acceptance is not a request"
        );
        let mut stranger = Directory::new();
        assert!(
            stranger.receive_request(&request).is_err(),
            "a request for an account this server does not hold is refused"
        );
    }

    #[test]
    fn replay_rebuilds_the_directory_and_refuses_a_broken_journal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::new();
        let (ann, _) = account_into(&mut directory, &path, "Ann");
        let (bo, _) = account_into(&mut directory, &path, "Bo");
        let (group, entry) = directory
            .create_group(&ann, "Crew", "hello", all())
            .unwrap();
        Directory::append(&path, &entry).unwrap();
        let entry = directory
            .set_member(
                Actor::Account(&ann),
                group,
                Member::Account { id: bo.clone() },
                Some(Level::Read),
            )
            .unwrap();
        Directory::append(&path, &entry).unwrap();
        let replayed = Directory::replay(&path).unwrap();
        assert_eq!(replayed.level_in(&bo, group), Some(Level::Read));
        assert_eq!(replayed.group(group).unwrap().description, "hello");

        let demote = DirectoryEntry::MemberSet {
            group,
            member: Member::Account { id: ann.clone() },
            level: Some(Level::Read),
        };
        let mut line = serde_json::to_string(&demote).unwrap();
        line.push('\n');
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&line);
        std::fs::write(&path, text).unwrap();
        assert!(
            Directory::replay(&path).is_err(),
            "removing the last admin is refused on replay"
        );
    }

    fn account_into(directory: &mut Directory, path: &Path, name: &str) -> (String, String) {
        let (credential, entry) = directory.sign_up(name).unwrap();
        Directory::append(path, &entry).unwrap();
        let id = directory.account_for(&credential).unwrap();
        (id, credential)
    }

    #[test]
    fn an_envelope_reattributed_to_another_account_does_not_verify() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (_, cy_credential) = account(&mut directory, "Cy");
        let (mut request, _) = directory.request_friend(&cy_credential, &bo, 1).unwrap();
        request.from = ann;
        assert!(request.verify().is_err());
    }

    #[test]
    fn an_open_directory_journals_accepted_changes_and_nothing_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, _) = directory.sign_up("Ann").unwrap();
        let ann = directory.account_for(&ann).unwrap();
        let (group, _) = directory.create_group(&ann, "Crew", "", all()).unwrap();
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        assert_eq!(lines, 2);
        assert!(
            directory
                .set_member(
                    Actor::Server,
                    group,
                    Member::Account { id: ann.clone() },
                    None
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().lines().count(),
            lines
        );
        let replayed = Directory::open(&path).unwrap();
        assert_eq!(replayed.level_in(&ann, group), Some(Level::Admin));
        assert_eq!(replayed.group(group).unwrap().name, "Crew");
    }

    #[test]
    fn malformed_hex_is_refused() {
        assert!(hex_to_array::<32>("zz").is_none());
        assert!(hex_to_array::<32>(&"0".repeat(63)).is_none());
    }
}
