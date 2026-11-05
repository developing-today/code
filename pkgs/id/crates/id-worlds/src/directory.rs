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

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;

use crate::artifact::{
    self, ArtifactLog, Document, FriendRemove, HomeDeclared, HomeServer, Kind, SignedArtifact,
    Stored, Tag,
};
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
    /// Manages members like `Manage`, and may set a world's isolation.
    Moderator,
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
    /// Whether an admin verified the account by hand.
    pub verified: bool,
    /// Further public keys that sign in as this account, besides its ID.
    pub keys: BTreeSet<String>,
    /// Confirmed email addresses, normalized to lower case.
    pub emails: BTreeSet<String>,
}

/// A request the directory refuses, carried in the error so transports can
/// choose a status without parsing messages.
///
/// The definition lives in `id-core` since the Phase 1 crate split
/// (#3447) — leaf modules there (petname) produce it too — and it is
/// re-exported here at its historical path.
pub use id_core::Refusal;

/// The envelope wire types live in `id-sync` (step 3c); this path is
/// the historical one callers use.
pub use id_sync::envelope::{Envelope, EnvelopeKind};

macro_rules! refuse {
    ($kind:ident, $($arg:tt)*) => {
        return Err(Refusal::$kind(format!($($arg)*)).into())
    };
}

macro_rules! refuse_unless {
    ($cond:expr, $kind:ident, $($arg:tt)*) => {
        if !$cond {
            refuse!($kind, $($arg)*);
        }
    };
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
    /// Whether anonymous viewers may see the group.
    pub public: bool,
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
    /// A new account whose ID is its own public key, so it has no secret.
    AccountKeyed {
        /// The account ID, a hex Ed25519 public key.
        id: String,
        /// The display name.
        name: String,
    },
    /// An admin verified an account by hand.
    AccountVerified {
        /// The account ID.
        id: String,
    },
    /// A public key was added to an account.
    AccountKeyAdded {
        /// The account ID.
        id: String,
        /// The hex public key.
        key: String,
    },
    /// A public key was removed from an account.
    AccountKeyRemoved {
        /// The account ID.
        id: String,
        /// The hex public key.
        key: String,
    },
    /// The owner of an email address proved it, and the address joined the account.
    AccountEmailConfirmed {
        /// The account ID.
        id: String,
        /// The normalized address.
        email: String,
    },
    /// An email address was removed from an account.
    AccountEmailRemoved {
        /// The account ID.
        id: String,
        /// The normalized address.
        email: String,
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
    /// A group's visibility to anonymous viewers changed.
    GroupVisibility {
        /// The group ID.
        id: u64,
        /// Whether anonymous viewers may see the group.
        public: bool,
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
        /// The envelope ID of the request, when it came in an envelope.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// A pending request was accepted.
    FriendAccepted {
        /// The account that made the request.
        from: String,
        /// The account that accepted it.
        to: String,
        /// The envelope ID of the request accepted. Must match the pending one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<String>,
        /// The envelope ID of this acceptance, when it came in an envelope.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// A friendship ended, and the requests it covers are void.
    FriendRemoved {
        /// One side of the friendship.
        a: String,
        /// The other side.
        b: String,
        /// Request IDs the removal voids. Empty for a removal that needs no request.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        requests: Vec<String>,
        /// Hash of the signed removal artifact this entry was applied
        /// with, when it was applied from one (#5624). Absent for a
        /// local removal (no artifact exists) and for entries journaled
        /// before this field was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<String>,
    },
    /// A friend-removal artifact was stored without an effect (#5624):
    /// the pair had no friendship or pending request it could end.
    /// Journaled at receipt so recovery can tell a processed artifact
    /// from an interrupted removal whose journal entry was lost.
    FriendRemovalNoted {
        /// One side of the pair the removal names.
        a: String,
        /// The other side of the pair the removal names.
        b: String,
        /// Request IDs the stored removal artifact names.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        requests: Vec<String>,
        /// Hash of the signed removal artifact that was noted.
        artifact: String,
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

/// Longest time after it is made that an envelope is accepted.
const ENVELOPE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// How far ahead of the receiving clock an envelope may be dated.
const ENVELOPE_SKEW_MS: u64 = 5 * 60 * 1000;

/// Friend requests one sender account may have pending on a server at once.
pub const MAX_PENDING_FRIEND_REQUESTS: usize = 20;

/// What receiving an envelope did to the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// The envelope changed the directory.
    Applied(DirectoryEntry),
    /// The envelope was already applied, so nothing changed.
    Duplicate,
}

/// The directory of accounts, groups, memberships and friends on one server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    accounts: BTreeMap<String, Account>,
    digests: BTreeMap<String, [u8; 32]>,
    groups: BTreeMap<u64, Group>,
    members: BTreeMap<(u64, Member), Level>,
    tiers: BTreeMap<Tier, WorldScopes>,
    friends: BTreeSet<(String, String)>,
    friend_requests: BTreeMap<(String, String), String>,
    requests: BTreeMap<(String, String), Option<String>>,
    removed_requests: BTreeSet<String>,
    seen: BTreeSet<String>,
    artifacts: ArtifactLog,
    next_group: u64,
    journal: Option<PathBuf>,
    /// Friend-removal journal entries seen during replay, in journal
    /// order (#5624): recovery matches stored removal artifacts against
    /// them to tell an already-applied removal from one whose journal
    /// entry was lost to a crash. The fourth element is the hash of the
    /// artifact the entry was applied with, when it was applied from
    /// one. Only replay fills this; it is not directory state.
    journaled_removals: Vec<(String, String, Vec<String>, Option<String>)>,
    /// Hashes of removal artifacts replay saw journaled as stored
    /// without an effect (`FriendRemovalNoted`, #5624): recovery must
    /// never apply them against state formed after they were stored.
    /// Only replay fills this; it is not directory state.
    noted_removal_artifacts: Vec<String>,
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
            friend_requests: BTreeMap::new(),
            requests: BTreeMap::new(),
            removed_requests: BTreeSet::new(),
            seen: BTreeSet::new(),
            artifacts: ArtifactLog::default(),
            next_group: 1,
            journal: None,
            journaled_removals: Vec::new(),
            noted_removal_artifacts: Vec::new(),
        }
    }

    /// Rebuild the directory from its journal and its artifact log, and
    /// journal every later change to it. A change is journaled before it
    /// becomes visible. A removal whose artifact was stored but whose journal
    /// entry was lost to a crash is applied again here.
    ///
    /// # Errors
    ///
    /// Fails if the journal or the artifact log cannot be replayed.
    pub fn open(path: &Path) -> Result<Self> {
        let mut directory = Self::replay(path)?;
        directory.artifacts = ArtifactLog::open(&artifact_path(path))?;
        directory.journal = Some(path.to_path_buf());
        let removals: Vec<(String, String, Vec<String>, String)> = directory
            .artifacts
            .all()
            .filter_map(|artifact| match artifact.parsed_for_replay() {
                Ok(Some(Kind::FriendRemove(removal))) => Some((
                    removal.a,
                    removal.b,
                    removal.requests,
                    artifact.hash.clone(),
                )),
                _ => None,
            })
            .collect();
        for (a, b, requests, hash) in removals {
            if directory.noted_removal_artifacts.contains(&hash) {
                continue;
            }
            let journaled = directory.take_journaled_removal(&a, &b, &requests, &hash);
            if let Some(entry) =
                directory.friend_removal_effect(&a, &b, &requests, journaled, &hash)
            {
                directory.apply(&entry)?;
            }
        }
        Ok(directory)
    }

    /// The signed artifacts this server holds.
    #[must_use]
    pub const fn artifacts(&self) -> &ArtifactLog {
        &self.artifacts
    }

    /// Rebuild the directory from its journal, one line per entry.
    ///
    /// # Errors
    ///
    /// Fails if a line is malformed or an entry breaks a directory rule.
    pub fn replay(path: &Path) -> Result<Self> {
        let mut directory = Self::new();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(directory),
            Err(error) => return Err(error.into()),
        };
        // #1521: an unterminated final line is a torn append — append
        // writes one line + newline in a single write_all + sync_data,
        // so a crash mid-append leaves a fragment the writer never
        // acknowledged. Mirror FileJournal::open (world_store.rs):
        // drop the fragment (parseable or not), warn, and truncate the
        // file to the last complete line. A newline-terminated line
        // that does not parse is corruption and still fails below.
        let kept_len = if bytes.is_empty() || bytes.ends_with(b"\n") {
            bytes.len()
        } else {
            bytes
                .iter()
                .rposition(|&byte| byte == b'\n')
                .map_or(0, |index| index + 1)
        };
        if kept_len < bytes.len() {
            tracing::warn!(
                "directory journal {}: dropping torn final entry at byte {kept_len}",
                path.display()
            );
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)?
                .set_len(u64::try_from(kept_len)?)?;
        }
        let text =
            std::str::from_utf8(&bytes[..kept_len]).context("directory journal is not UTF-8")?;
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: DirectoryEntry = serde_json::from_str(line)
                .with_context(|| format!("directory journal line {} is malformed", number + 1))?;
            directory
                .apply_replay(&entry)
                .with_context(|| format!("directory journal line {} is refused", number + 1))?;
            match &entry {
                DirectoryEntry::FriendRemoved {
                    a,
                    b,
                    requests,
                    artifact,
                } => {
                    directory.journaled_removals.push((
                        a.clone(),
                        b.clone(),
                        requests.clone(),
                        artifact.clone(),
                    ));
                }
                DirectoryEntry::FriendRemovalNoted { artifact, .. } => {
                    directory.noted_removal_artifacts.push(artifact.clone());
                }
                _ => {}
            }
        }
        Ok(directory)
    }

    /// Rebuild the directory from complete journal lines without modifying
    /// the file. Used by read-only world replicas: the normal replay path
    /// truncates a torn final line as writer recovery, but opening a replica
    /// must not rewrite even a damaged directory sidecar.
    ///
    /// A torn final fragment is ignored (like [`Self::replay`]); a malformed
    /// newline-terminated entry remains fatal. The artifact log is verified
    /// without repair, and a signed friend removal that was stored before a
    /// crash but not journaled is applied in memory (like writer open).
    pub fn replay_read_only(path: &Path) -> Result<Self> {
        let mut directory = Self::new();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let kept_len = bytes
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |index| index + 1);
        let text =
            std::str::from_utf8(&bytes[..kept_len]).context("directory journal is not UTF-8")?;
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: DirectoryEntry = serde_json::from_str(line)
                .with_context(|| format!("directory journal line {} is malformed", number + 1))?;
            directory
                .apply_replay(&entry)
                .with_context(|| format!("directory journal line {} is refused", number + 1))?;
            match &entry {
                DirectoryEntry::FriendRemoved {
                    a,
                    b,
                    requests,
                    artifact,
                } => {
                    directory.journaled_removals.push((
                        a.clone(),
                        b.clone(),
                        requests.clone(),
                        artifact.clone(),
                    ));
                }
                DirectoryEntry::FriendRemovalNoted { artifact, .. } => {
                    directory.noted_removal_artifacts.push(artifact.clone());
                }
                _ => {}
            }
        }
        let artifacts = ArtifactLog::open_read_only(&artifact_path(path))?;
        let removals: Vec<(String, String, Vec<String>, String)> = artifacts
            .all()
            .filter_map(|artifact| match artifact.parsed_for_replay() {
                Ok(Some(Kind::FriendRemove(removal))) => Some((
                    removal.a,
                    removal.b,
                    removal.requests,
                    artifact.hash.clone(),
                )),
                _ => None,
            })
            .collect();
        for (a, b, requests, hash) in removals {
            if directory.noted_removal_artifacts.contains(&hash) {
                continue;
            }
            let journaled = directory.take_journaled_removal(&a, &b, &requests, &hash);
            if let Some(entry) =
                directory.friend_removal_effect(&a, &b, &requests, journaled, &hash)
            {
                directory.apply_replay(&entry)?;
            }
        }
        directory.artifacts = artifacts;
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
        refuse_unless!(
            actor == Actor::Server,
            Forbidden,
            "only the server verifies accounts"
        );
        let entry = DirectoryEntry::AccountVerified { id: id.to_owned() };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Create an account whose ID is the caller's public key. It has no
    /// secret: the key proves itself on each sign-in.
    ///
    /// # Errors
    ///
    /// Fails if the key is not an Ed25519 public key, the name is invalid, or
    /// the key already belongs to an account.
    pub fn sign_up_keyed(&mut self, key: &str, name: &str) -> Result<DirectoryEntry> {
        let entry = DirectoryEntry::AccountKeyed {
            id: normalize_key(key)?,
            name: name.to_owned(),
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Add a public key an account signs in with. The directory cannot see
    /// the key's private half, so the caller must have proven the key on the
    /// request that adds it.
    ///
    /// # Errors
    ///
    /// Fails for another account's actor, a malformed key, or a key held by
    /// any account already.
    pub fn add_key(
        &mut self,
        actor: Actor<'_>,
        account: &str,
        key: &str,
    ) -> Result<DirectoryEntry> {
        refuse_unless!(
            may_act_for(actor, account),
            Forbidden,
            "only the account itself adds keys"
        );
        let entry = DirectoryEntry::AccountKeyAdded {
            id: account.to_owned(),
            key: normalize_key(key)?,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Remove a public key an account added. Its own ID stays.
    ///
    /// # Errors
    ///
    /// Fails for another account's actor, or if the account does not hold
    /// that key.
    pub fn remove_key(
        &mut self,
        actor: Actor<'_>,
        account: &str,
        key: &str,
    ) -> Result<DirectoryEntry> {
        refuse_unless!(
            may_act_for(actor, account),
            Forbidden,
            "only the account itself removes keys"
        );
        let entry = DirectoryEntry::AccountKeyRemoved {
            id: account.to_owned(),
            key: normalize_key(key)?,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Attach an email address to an account once its owner has proved it.
    /// Only the server calls this, after a confirmation code matched.
    ///
    /// # Errors
    ///
    /// Fails for account actors, a malformed address, or an address another
    /// account holds.
    pub fn confirm_email(
        &mut self,
        actor: Actor<'_>,
        account: &str,
        address: &str,
    ) -> Result<DirectoryEntry> {
        refuse_unless!(
            actor == Actor::Server,
            Forbidden,
            "only the server confirms addresses"
        );
        let entry = DirectoryEntry::AccountEmailConfirmed {
            id: account.to_owned(),
            email: normalize_email(address)?,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Remove an email address from an account.
    ///
    /// # Errors
    ///
    /// Fails for another account's actor, or if the account does not hold
    /// that address.
    pub fn remove_email(
        &mut self,
        actor: Actor<'_>,
        account: &str,
        address: &str,
    ) -> Result<DirectoryEntry> {
        refuse_unless!(
            may_act_for(actor, account),
            Forbidden,
            "only the account itself removes addresses"
        );
        let entry = DirectoryEntry::AccountEmailRemoved {
            id: account.to_owned(),
            email: normalize_email(address)?,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// The account a public key signs in as: its own ID, or a key it added.
    #[must_use]
    pub fn account_for_key(&self, key: &str) -> Option<String> {
        let key = normalize_key(key).ok()?;
        self.accounts
            .values()
            .find(|account| account.id == key || account.keys.contains(&key))
            .map(|account| account.id.clone())
    }

    /// The account that confirmed an email address, if any.
    #[must_use]
    pub fn account_for_email(&self, address: &str) -> Option<String> {
        let address = normalize_email(address).ok()?;
        self.accounts
            .values()
            .find(|account| account.emails.contains(&address))
            .map(|account| account.id.clone())
    }

    /// Whether an account is verified: by an admin, or by a confirmed email.
    #[must_use]
    pub fn is_verified(&self, account: &str) -> bool {
        self.accounts
            .get(account)
            .is_some_and(|found| found.verified || !found.emails.is_empty())
    }

    /// Ask `to` to be friends on behalf of an account the caller has
    /// authenticated some other way than a credential.
    ///
    /// # Errors
    ///
    /// Fails for a request to oneself, an existing friendship, or a pending
    /// request in either direction.
    pub fn request_friend_as(&mut self, me: &str, to: &str) -> Result<DirectoryEntry> {
        let entry = DirectoryEntry::FriendRequested {
            from: me.to_owned(),
            to: to.to_owned(),
            id: None,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// Accept the pending request from `from` on behalf of an authenticated account.
    ///
    /// # Errors
    ///
    /// Fails when no request from `from` is pending.
    pub fn accept_friend_as(&mut self, me: &str, from: &str) -> Result<DirectoryEntry> {
        let entry = DirectoryEntry::FriendAccepted {
            from: from.to_owned(),
            to: me.to_owned(),
            request: self.pending_id(from, me),
            id: None,
        };
        self.apply(&entry)?;
        Ok(entry)
    }

    /// End a friendship on behalf of an authenticated account.
    ///
    /// # Errors
    ///
    /// Fails when the two are not friends.
    pub fn remove_friend_as(&mut self, me: &str, other: &str) -> Result<DirectoryEntry> {
        let entry = DirectoryEntry::FriendRemoved {
            a: me.to_owned(),
            b: other.to_owned(),
            requests: self.friendship_requests(me, other),
            artifact: None,
        };
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
        refuse_unless!(
            actor == Actor::Server,
            Forbidden,
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
        refuse_unless!(
            self.allowed(actor, group, needed),
            Forbidden,
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

    /// Show a group to anonymous viewers, or hide it. Admins only.
    ///
    /// # Errors
    ///
    /// Fails unless the actor is an admin of the group.
    pub fn set_public(
        &mut self,
        actor: Actor<'_>,
        group: u64,
        public: bool,
    ) -> Result<DirectoryEntry> {
        refuse_unless!(
            self.allowed(actor, group, Level::Admin),
            Forbidden,
            "only an admin changes who can see a group"
        );
        let entry = DirectoryEntry::GroupVisibility { id: group, public };
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
        refuse_unless!(
            self.allowed(actor, group, Level::Admin),
            Forbidden,
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
        refuse_unless!(
            self.allowed(actor, group, Level::Manage),
            Forbidden,
            "not allowed to manage this group"
        );
        if let Actor::Account(me) = actor
            && self.level_in(me, group) != Some(Level::Admin)
        {
            refuse_unless!(
                member != (Member::Account { id: me.to_owned() }),
                Forbidden,
                "you cannot change your own level"
            );
            refuse_unless!(
                level.is_none_or(|level| level <= Level::Write),
                Forbidden,
                "managers can grant up to write"
            );
            let current = self.members.get(&(group, member.clone())).copied();
            refuse_unless!(
                current.is_none_or(|current| current < Level::Manage),
                Forbidden,
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

    /// Whether an account holds `moderator` or better in some group, which is
    /// what lets it change a world's isolation.
    #[must_use]
    pub fn is_moderator(&self, account: &str) -> bool {
        self.groups.keys().any(|group| {
            self.level_in(account, *group)
                .is_some_and(|level| level >= Level::Moderator)
        })
    }

    /// The level an account holds in a group, directly or through nested
    /// groups. A nested group passes on at most the level it is held at.
    #[must_use]
    pub fn level_in(&self, account: &str, group: u64) -> Option<Level> {
        self.level_in_cached(account, group, &mut BTreeMap::new())
    }

    fn level_in_cached(
        &self,
        account: &str,
        group: u64,
        cache: &mut BTreeMap<u64, Option<Level>>,
    ) -> Option<Level> {
        if let Some(level) = cache.get(&group) {
            return *level;
        }
        let direct = self
            .members
            .get(&(
                group,
                Member::Account {
                    id: account.to_owned(),
                },
            ))
            .copied();
        let best = self
            .members
            .range((group, Member::Group { id: 0 })..=(group, Member::Group { id: u64::MAX }))
            .fold(direct, |best, ((_, member), held_at)| match member {
                Member::Group { id } => match self.level_in_cached(account, *id, cache) {
                    Some(inner) => best.max(Some(inner.min(*held_at))),
                    None => best,
                },
                Member::Account { .. } => best,
            });
        cache.insert(group, best);
        best
    }

    /// Every permission an account may use: its tier's set, plus the set of
    /// each group it belongs to. A capability's own scopes still narrow this.
    #[must_use]
    pub fn ceiling(&self, account: &str) -> WorldScopes {
        let tier = match self.accounts.get(account) {
            Some(_) if self.is_verified(account) => Tier::Verified,
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

    /// Directory facts a read-only replica needs for spectator
    /// authorization (F12.13 phase 4): everything
    /// [`crate::world::WorldCore::effective_scopes`] consults for the given
    /// capability subjects — account ceilings (tiers and group grants) and
    /// the friendship graph — as entries the replica applies to an empty
    /// directory. The caller passes every account appearing in the
    /// replicated journal's capability chains; unknown subjects are skipped
    /// (they read back under the anonymous tier, fail-closed).
    ///
    /// Account secrets never cross: signed-up accounts carry only their
    /// stored digest, keyed accounts only their public ID. Friendships are
    /// re-expressed without envelope IDs — the request/accept pair
    /// validates as a unit, and only current friendships are exported, so
    /// there is nothing to void. Groups export only when the groups a
    /// subject's level flows through form a contiguous `1..=k` prefix (the
    /// journal replays groups in creation order); group founders are the
    /// lowest-ID account already holding admin, so no authority widens.
    ///
    /// # Errors
    ///
    /// Fails when a group the subjects depend on cannot be represented as
    /// such a prefix, so a gappy topology can never silently narrow a
    /// subject's ceiling. Worlds without group grants (like the phase-4
    /// friendship-delegation acceptance case) never hit this.
    pub fn replica_facts_for(&self, subjects: &BTreeSet<String>) -> Result<Vec<DirectoryEntry>> {
        // Groups that grant a subject some level, closed under nesting: a
        // subject's level can flow through a nested group, and `MemberSet`
        // refuses an unknown group.
        let mut groups: BTreeSet<u64> = BTreeSet::new();
        let mut stack: Vec<u64> = self
            .groups
            .keys()
            .filter(|id| subjects.iter().any(|s| self.level_in(s, **id).is_some()))
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            if groups.insert(id) {
                for ((_, member), _) in self
                    .members
                    .range((id, Member::Group { id: 0 })..=(id, Member::Group { id: u64::MAX }))
                {
                    if let Member::Group { id: nested } = member {
                        stack.push(*nested);
                    }
                }
            }
        }
        // Journal replay creates groups in order, so the export must be the
        // contiguous prefix starting at group 1.
        let ordered: Vec<u64> = groups.iter().copied().collect();
        let contiguous = ordered
            .iter()
            .enumerate()
            .all(|(index, id)| u64::try_from(index + 1).is_ok_and(|expected| expected == *id));
        ensure!(
            contiguous,
            "replica export cannot represent non-contiguous group IDs {ordered:?}"
        );
        // Accounts to reproduce: the subjects that exist, plus founders of
        // exported groups (`GroupCreated` refuses an unknown founder).
        let mut accounts: BTreeSet<String> = subjects
            .iter()
            .filter(|id| self.accounts.contains_key(*id))
            .cloned()
            .collect();
        let mut founders: BTreeMap<u64, String> = BTreeMap::new();
        for id in &ordered {
            // Admin can be inherited through nested-group membership; use
            // the same effective-level calculation as `ensure_admins`.
            let founder = self
                .accounts
                .keys()
                .filter(|account| self.level_in(account.as_str(), *id) == Some(Level::Admin))
                .min()
                .cloned();
            let Some(founder) = founder else {
                anyhow::bail!("replica export cannot represent group {id} with no account admin");
            };
            accounts.insert(founder.clone());
            founders.insert(*id, founder);
        }
        let mut entries = Vec::new();
        let mut sorted_accounts: Vec<&String> = accounts.iter().collect();
        sorted_accounts.sort();
        for id in sorted_accounts {
            let account = &self.accounts[id.as_str()];
            if let Some(digest) = self.digests.get(id.as_str()) {
                entries.push(DirectoryEntry::AccountSignedUp {
                    id: (*id).clone(),
                    name: account.name.clone(),
                    digest: hex_encode(digest),
                });
            } else {
                entries.push(DirectoryEntry::AccountKeyed {
                    id: (*id).clone(),
                    name: account.name.clone(),
                });
            }
            // Ceilings read the verified tier (directly verified or via a
            // confirmed email), so export the verified flag on that basis:
            // the replica reaches the same ceiling either way.
            if self.is_verified(id) {
                entries.push(DirectoryEntry::AccountVerified { id: (*id).clone() });
            }
        }
        // Non-default tiers ride along: ceilings intersect them.
        let defaults = BTreeMap::from([
            (Tier::Anonymous, WorldScopes::GUEST),
            (Tier::Registered, WorldScopes::GUEST),
            (
                Tier::Verified,
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
            ),
        ]);
        for (tier, scopes) in &self.tiers {
            if defaults.get(tier) != Some(scopes) {
                entries.push(DirectoryEntry::TierSet {
                    tier: *tier,
                    scopes: scopes.bits(),
                });
            }
        }
        for id in &ordered {
            let group = &self.groups[id];
            entries.push(DirectoryEntry::GroupCreated {
                id: *id,
                name: group.name.clone(),
                description: group.description.clone(),
                scopes: group.scopes.bits(),
                founder: founders[id].clone(),
            });
        }
        // All groups exist before any nested `MemberSet` references them.
        // Directory replay refuses a membership naming a not-yet-created
        // group, even when both IDs belong to one contiguous replica prefix.
        // Founder downgrades are collected here and emitted only after
        // every group's memberships exist (see the deferral below).
        let mut founder_downgrades: Vec<DirectoryEntry> = Vec::new();
        for id in &ordered {
            // Subject memberships reproduce subject ceilings; nested links
            // reproduce the paths levels flow through. Other accounts stay
            // out: the founder above already keeps an admin in place.
            let mut memberships: Vec<(Member, Level)> = self
                .members
                .range(
                    (*id, Member::Account { id: String::new() })
                        ..=(*id, Member::Group { id: u64::MAX }),
                )
                .map(|((_, member), level)| (member.clone(), *level))
                .filter(|(member, _)| match member {
                    Member::Account { id } => subjects.contains(id),
                    Member::Group { .. } => true,
                })
                .collect();
            memberships.sort_by(|a, b| a.0.cmp(&b.0));
            for (member, level) in memberships {
                // The founder's admin comes from `GroupCreated` itself.
                if member
                    == (Member::Account {
                        id: founders[id].clone(),
                    })
                    && level == Level::Admin
                {
                    continue;
                }
                let is_founder = member
                    == (Member::Account {
                        id: founders[id].clone(),
                    });
                let entry = DirectoryEntry::MemberSet {
                    group: *id,
                    member,
                    level: Some(level),
                };
                // A founder whose direct level is below `Admin` was chosen
                // because they hold the group's admin through an inherited
                // path. Sorted order emits account memberships before
                // nested-group ones, so the downgrade could land before
                // the nested membership carrying that path exists, leaving
                // the group momentarily admin-less — `checked_mode`
                // rejects it through `ensure_admins` and replica startup
                // fails. Defer founder downgrades until all memberships
                // are in place.
                if is_founder {
                    founder_downgrades.push(entry);
                } else {
                    entries.push(entry);
                }
            }
        }
        entries.extend(founder_downgrades);
        // Current friendships among the exported accounts. Preserve the
        // request ID when one exists: a signed FriendRemove artifact uses
        // it to distinguish this friendship from an older relationship
        // between the same pair after the directory is projected.
        let sorted: Vec<&String> = accounts.iter().collect();
        let mut pairs: Vec<(&String, &String)> = Vec::new();
        for (index, a) in sorted.iter().enumerate() {
            for b in &sorted[index + 1..] {
                if self.are_friends(a, b) {
                    pairs.push((a, b));
                }
            }
        }
        for (a, b) in pairs {
            let request = self.friend_requests.get(&pair(a, b)).cloned();
            entries.push(DirectoryEntry::FriendRequested {
                from: (*a).clone(),
                to: (*b).clone(),
                id: request.clone(),
            });
            entries.push(DirectoryEntry::FriendAccepted {
                from: (*a).clone(),
                to: (*b).clone(),
                request,
                id: None,
            });
        }
        Ok(entries)
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

    /// Every account, in ID order.
    pub fn accounts(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }

    /// Every group, in ID order.
    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.groups.values()
    }

    /// Pending friend requests to and from an account, as (incoming, outgoing).
    #[must_use]
    pub fn pending_for(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let incoming = self
            .requests
            .keys()
            .filter(|(_, to)| to == id)
            .map(|(from, _)| from.clone())
            .collect();
        let outgoing = self
            .requests
            .keys()
            .filter(|(from, _)| from == id)
            .map(|(_, to)| to.clone())
            .collect();
        (incoming, outgoing)
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
    /// `audience` is the world ID of the server `to` lives on.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential, a request to oneself, an existing
    /// friendship, or a pending request in either direction.
    pub fn request_friend(
        &mut self,
        credential: &str,
        to: &str,
        audience: &str,
        at: u64,
    ) -> Result<(Envelope, DirectoryEntry)> {
        let (envelope, entry) = self.prepare_request_friend(credential, to, audience, at)?;
        self.apply(&entry)?;
        Ok((envelope, entry))
    }

    /// Sign a friend request and check that it would apply, without applying
    /// it. Apply the returned entry with [`Self::apply`] once the envelope is
    /// safely queued.
    ///
    /// # Errors
    ///
    /// Fails as [`Self::request_friend`] does.
    pub fn prepare_request_friend(
        &self,
        credential: &str,
        to: &str,
        audience: &str,
        at: u64,
    ) -> Result<(Envelope, DirectoryEntry)> {
        let (from, key) = self.authenticate(credential)?;
        let envelope = Envelope::signed(
            &key,
            EnvelopeKind::FriendRequest,
            &from,
            to,
            audience,
            None,
            at,
        )?;
        let entry = DirectoryEntry::FriendRequested {
            from,
            to: to.to_owned(),
            id: Some(envelope.id.clone()),
        };
        self.checked(&entry)?;
        Ok((envelope, entry))
    }

    /// Accept the pending request from `from`, returning the signed acceptance.
    /// `audience` is the world ID of the server `from` lives on.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential, when no request is pending, or when
    /// the pending request has no envelope ID to name.
    pub fn accept_friend(
        &mut self,
        credential: &str,
        from: &str,
        audience: &str,
        at: u64,
    ) -> Result<(Envelope, DirectoryEntry)> {
        let (envelope, entry) = self.prepare_accept_friend(credential, from, audience, at)?;
        self.apply(&entry)?;
        Ok((envelope, entry))
    }

    /// Sign an acceptance and check that it would apply, without applying it.
    /// Apply the returned entry with [`Self::apply`] once the envelope is safely
    /// queued.
    ///
    /// # Errors
    ///
    /// Fails as [`Self::accept_friend`] does.
    pub fn prepare_accept_friend(
        &self,
        credential: &str,
        from: &str,
        audience: &str,
        at: u64,
    ) -> Result<(Envelope, DirectoryEntry)> {
        let (me, key) = self.authenticate(credential)?;
        let Some(request) = self.pending_id(from, &me) else {
            bail!("no pending request from {from}");
        };
        let envelope = Envelope::signed(
            &key,
            EnvelopeKind::FriendAccept,
            &me,
            from,
            audience,
            Some(&request),
            at,
        )?;
        let entry = DirectoryEntry::FriendAccepted {
            from: from.to_owned(),
            to: me,
            request: Some(request),
            id: Some(envelope.id.clone()),
        };
        self.checked(&entry)?;
        Ok((envelope, entry))
    }

    /// Apply an envelope another server signed, for a local account. It is
    /// refused unless its signature verifies, it is for `audience`, it is
    /// dated within the expiry window, and its target is a local account. A
    /// stale acceptance names a request that is no longer pending and is
    /// refused. An envelope already applied is reported as a duplicate.
    ///
    /// # Errors
    ///
    /// Fails with a [`Refusal`] for an envelope this server must not apply.
    pub fn receive(&mut self, envelope: &Envelope, audience: &str, now: u64) -> Result<Received> {
        envelope
            .verify()
            .map_err(|error| Refusal::Unauthenticated(format!("{error:#}")))?;
        refuse_unless!(
            envelope.audience == audience,
            Forbidden,
            "envelope is for another server"
        );
        if self.seen.contains(&envelope.id) {
            return Ok(Received::Duplicate);
        }
        // #5624: a friend request's ID is persisted verbatim and later
        // named by friend_remove artifacts, whose receipt contract
        // (#3146) only accepts the canonical shape the producer mints
        // (16 bytes as 32 lowercase hex). Refuse a noncanonical ID here,
        // before it can be persisted and strand the friendship. The
        // shape check follows the duplicate lookup above so an already
        // applied legacy envelope still reports Duplicate, not Forbidden.
        if matches!(envelope.kind, EnvelopeKind::FriendRequest) {
            refuse_unless!(
                artifact::is_friend_request_id(&envelope.id),
                Forbidden,
                "friend request ID {} is not 32 lowercase hexadecimal characters",
                envelope.id
            );
        }
        refuse_unless!(
            envelope.at <= now.saturating_add(ENVELOPE_SKEW_MS),
            Forbidden,
            "envelope is dated in the future"
        );
        refuse_unless!(
            now.saturating_sub(envelope.at) <= ENVELOPE_TTL_MS,
            Forbidden,
            "envelope has expired"
        );
        refuse_unless!(
            self.accounts.contains_key(&envelope.to),
            NotFound,
            "envelope is for no account on this server"
        );
        if matches!(envelope.kind, EnvelopeKind::FriendRequest) {
            refuse_unless!(
                self.pending_from(&envelope.from) < MAX_PENDING_FRIEND_REQUESTS,
                Forbidden,
                "{} already has {MAX_PENDING_FRIEND_REQUESTS} friend requests pending on this server",
                envelope.from
            );
        }
        let entry = match (envelope.kind, &envelope.request) {
            (EnvelopeKind::FriendRequest, None) => DirectoryEntry::FriendRequested {
                from: envelope.from.clone(),
                to: envelope.to.clone(),
                id: Some(envelope.id.clone()),
            },
            (EnvelopeKind::FriendAccept, Some(request)) => DirectoryEntry::FriendAccepted {
                from: envelope.to.clone(),
                to: envelope.from.clone(),
                request: Some(request.clone()),
                id: Some(envelope.id.clone()),
            },
            _ => refuse!(Forbidden, "envelope names a request only as an acceptance"),
        };
        self.apply(&entry)?;
        Ok(Received::Applied(entry))
    }

    fn pending_from(&self, from: &str) -> usize {
        self.requests
            .keys()
            .filter(|(sender, _)| sender == from)
            .count()
    }

    fn pending_id(&self, from: &str, to: &str) -> Option<String> {
        self.requests
            .get(&(from.to_owned(), to.to_owned()))
            .cloned()
            .flatten()
    }

    /// End a friendship.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential or when the two are not friends.
    pub fn remove_friend(&mut self, credential: &str, other: &str) -> Result<SignedArtifact> {
        let (me, key) = self.authenticate(credential)?;
        let stored = self.friendship_requests(&me, other);
        // #5624 legacy path: a request ID persisted before admission
        // validation may be noncanonical. The signed artifact carries
        // only canonical IDs, so the #3146 receipt contract is not
        // weakened and receiving servers still apply the removal (the
        // friendship itself ends by pair, not by request ID). The local
        // entry below carries every stored ID verbatim, so the legacy
        // IDs are voided in `removed_requests` exactly like canonical
        // ones instead of making the removal unrepresentable.
        let removal = FriendRemove {
            a: me.clone(),
            b: other.to_owned(),
            requests: stored
                .iter()
                .filter(|id| artifact::is_friend_request_id(id))
                .cloned()
                .collect(),
        };
        let artifact = self.artifacts.sign(&key, &Kind::FriendRemove(removal))?;
        let entry = DirectoryEntry::FriendRemoved {
            a: me,
            b: other.to_owned(),
            requests: stored,
            artifact: Some(artifact.hash.clone()),
        };
        self.store_artifact(&artifact, Some(&entry))?;
        Ok(artifact)
    }

    /// Publish the servers an account's artifacts are held on. A newer
    /// declaration replaces an older one.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential or a declaration that is malformed.
    pub fn declare_home(
        &mut self,
        credential: &str,
        servers: Vec<HomeServer>,
    ) -> Result<SignedArtifact> {
        let (account, _) = self.authenticate(credential)?;
        let declared = HomeDeclared { account, servers };
        // Refuse here what receipt would refuse (#3187): the same check
        // `SignedArtifact::parsed` runs, before anything is signed.
        artifact::check_home_declared(&declared.account, &declared, artifact::ParseMode::Strict)?;
        self.publish(credential, &Kind::HomeDeclared(declared), Vec::new())
    }

    /// Publish structured JSON as the signed document of the credential's account.
    /// `after` names artifacts the account had applied before this one.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential, or a document that is too large or malformed.
    pub fn publish_document(
        &mut self,
        credential: &str,
        media_type: &str,
        content: serde_json::Value,
        after: Vec<String>,
    ) -> Result<SignedArtifact> {
        let document = Document {
            media_type: media_type.to_owned(),
            content,
        };
        self.publish(credential, &Kind::Document(document), after)
    }

    /// Label an artifact, by its hash, as the credential's account.
    ///
    /// # Errors
    ///
    /// Fails for an invalid credential, or a tag that is malformed.
    pub fn publish_tag(
        &mut self,
        credential: &str,
        subject: &str,
        name: &str,
        value: Option<String>,
        after: Vec<String>,
    ) -> Result<SignedArtifact> {
        let tag = Tag {
            subject: subject.to_owned(),
            name: name.to_owned(),
            value,
        };
        self.publish(credential, &Kind::Tag(tag), after)
    }

    fn publish(
        &mut self,
        credential: &str,
        kind: &Kind,
        after: Vec<String>,
    ) -> Result<SignedArtifact> {
        let (_, key) = self.authenticate(credential)?;
        let artifact = self.artifacts.sign_after(&key, kind, after)?;
        self.store_artifact(&artifact, None)?;
        Ok(artifact)
    }

    /// Store an artifact another server signed, and apply what it says. A
    /// removal is applied only where it has an effect; a removal naming no
    /// request for a pair this server never befriended is stored alone.
    ///
    /// # Errors
    ///
    /// Fails with a [`Refusal`] for an artifact that does not verify, extends
    /// the chain wrongly, or breaks a directory rule.
    pub fn receive_artifact(&mut self, artifact: &SignedArtifact) -> Result<Stored> {
        self.receive_artifact_with_outcome(artifact)
            .map(|(stored, _legacy_note)| stored)
    }

    pub(crate) fn receive_artifact_with_outcome(
        &mut self,
        artifact: &SignedArtifact,
    ) -> Result<(Stored, Option<artifact::LegacyArtifactNote>)> {
        let mut next = self.clone();
        let (stored, parsed, legacy_note) = next.artifacts.insert_received(artifact.clone())?;
        if stored == Stored::Duplicate {
            return Ok((stored, legacy_note));
        }
        // Preserve the #5624 distinction between a removal artifact that
        // was fully processed with no effect and one whose journal entry
        // was lost. `parsed` is the already-validated receive result, so
        // this also works for a version-scoped legacy receipt.
        let no_effect_removal = match parsed.as_ref() {
            Some(Kind::FriendRemove(removal)) => Some(DirectoryEntry::FriendRemovalNoted {
                a: removal.a.clone(),
                b: removal.b.clone(),
                requests: removal.requests.clone(),
                artifact: artifact.hash.clone(),
            }),
            _ => None,
        };
        let effect = match parsed {
            Some(Kind::FriendRemove(removal)) => {
                let requests = next.pair_bound_requests(&removal.a, &removal.b, &removal.requests);
                if next.are_friends(&removal.a, &removal.b) || !requests.is_empty() {
                    // #5624: a legacy removal artifact names no requests
                    // at all; void the pair's stored IDs locally exactly
                    // like the producer's own entry does, instead of
                    // leaving the legacy formed-by ID unrecorded.
                    let entry_requests = if requests.is_empty() && removal.requests.is_empty() {
                        next.friendship_requests(&removal.a, &removal.b)
                    } else {
                        requests
                    };
                    Some(DirectoryEntry::FriendRemoved {
                        a: removal.a,
                        b: removal.b,
                        requests: entry_requests,
                        artifact: Some(artifact.hash.clone()),
                    })
                } else if removal.requests.is_empty() {
                    // #5624: a pending legacy request cannot be named by
                    // the artifact either, so an empty-request removal
                    // cancels this pair's locally held legacy pending
                    // IDs; a canonical pending request is untouched (a
                    // removal of it would name it).
                    let legacy = next.legacy_pair_requests(&removal.a, &removal.b);
                    if legacy.is_empty() {
                        None
                    } else {
                        Some(DirectoryEntry::FriendRemoved {
                            a: removal.a,
                            b: removal.b,
                            requests: legacy,
                            artifact: Some(artifact.hash.clone()),
                        })
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        // A no-effect removal is journaled as noted, preserving replay's
        // knowledge that it was processed at receipt rather than lost.
        let effect = effect.or(no_effect_removal);
        if let Some(entry) = effect.as_ref() {
            next.change(entry)?;
            next.ensure_admins()?;
        }
        if let Some(path) = &self.journal {
            artifact::append(&artifact_path(path), artifact)?;
            if let Some(entry) = effect.as_ref() {
                Self::append(path, entry)?;
            }
        }
        *self = next;
        Ok((stored, legacy_note))
    }

    fn store_artifact(
        &mut self,
        artifact: &SignedArtifact,
        effect: Option<&DirectoryEntry>,
    ) -> Result<Stored> {
        let mut next = self.clone();
        let stored = next.artifacts.insert(artifact.clone())?;
        if stored == Stored::Duplicate {
            return Ok(stored);
        }
        if let Some(entry) = effect {
            next.change(entry)?;
            next.ensure_admins()?;
        }
        if let Some(path) = &self.journal {
            artifact::append(&artifact_path(path), artifact)?;
            if let Some(entry) = effect {
                Self::append(path, entry)?;
            }
        }
        *self = next;
        Ok(stored)
    }

    fn authenticate(&self, credential: &str) -> Result<(String, SigningKey)> {
        let refused = || {
            anyhow::Error::from(Refusal::Unauthenticated(
                "invalid account credential".to_owned(),
            ))
        };
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

    fn insert_account(&mut self, id: &str, name: &str, refuse_format: bool) -> Result<()> {
        self.accounts.insert(
            id.to_owned(),
            Account {
                id: id.to_owned(),
                name: clean_name_mode(name, refuse_format)?,
                verified: false,
                keys: BTreeSet::new(),
                emails: BTreeSet::new(),
            },
        );
        Ok(())
    }

    fn allowed(&self, actor: Actor<'_>, group: u64, needed: Level) -> bool {
        match actor {
            Actor::Server => true,
            Actor::Account(id) => self
                .level_in(id, group)
                .is_some_and(|level| level >= needed),
        }
    }

    pub fn apply(&mut self, entry: &DirectoryEntry) -> Result<()> {
        self.apply_mode(entry, true)
    }

    /// Replay a historical journal entry. Entries accepted before the
    /// category-Cf refusal (#1778) may carry format characters; replaying
    /// them must not refuse, or a world whose journal predates the fix
    /// can never reopen. New writes still go through `apply` and refuse.
    fn apply_replay(&mut self, entry: &DirectoryEntry) -> Result<()> {
        self.apply_mode(entry, false)
    }

    /// Apply a fact projected from the writer's already-admitted directory
    /// to a read-only replica. It must use the same historical validation
    /// as replay, not new-write validation: an old group name containing a
    /// format character was valid when written and must remain loadable.
    pub(crate) fn apply_replica_fact(&mut self, entry: &DirectoryEntry) -> Result<()> {
        self.apply_replay(entry)
    }

    fn apply_mode(&mut self, entry: &DirectoryEntry, refuse_format: bool) -> Result<()> {
        let next = self.checked_mode(entry, refuse_format)?;
        if let Some(path) = &self.journal {
            Self::append(path, entry)?;
        }
        *self = next;
        Ok(())
    }

    fn checked(&self, entry: &DirectoryEntry) -> Result<Self> {
        self.checked_mode(entry, true)
    }

    fn checked_mode(&self, entry: &DirectoryEntry, refuse_format: bool) -> Result<Self> {
        let mut next = self.clone();
        next.change_mode(entry, refuse_format)?;
        next.ensure_admins()?;
        Ok(next)
    }

    fn change(&mut self, entry: &DirectoryEntry) -> Result<()> {
        self.change_mode(entry, true)
    }

    fn change_mode(&mut self, entry: &DirectoryEntry, refuse_format: bool) -> Result<()> {
        match entry {
            DirectoryEntry::AccountSignedUp { id, name, digest } => {
                ensure_account_id(id)?;
                refuse_unless!(
                    !self.accounts.contains_key(id),
                    Conflict,
                    "account already exists"
                );
                let digest = hex_to_array::<32>(digest).context("malformed account digest")?;
                self.insert_account(id, name, refuse_format)?;
                self.digests.insert(id.clone(), digest);
            }
            DirectoryEntry::AccountKeyed { id, name } => {
                ensure_account_id(id)?;
                refuse_unless!(
                    self.account_for_key(id).is_none(),
                    Conflict,
                    "that key already belongs to an account"
                );
                self.insert_account(id, name, refuse_format)?;
            }
            DirectoryEntry::AccountVerified { id } => {
                self.accounts
                    .get_mut(id)
                    .context("no such account")?
                    .verified = true;
            }
            DirectoryEntry::AccountKeyAdded { id, key } => {
                ensure_account_id(key)?;
                refuse_unless!(
                    self.account_for_key(key).is_none(),
                    Conflict,
                    "that key already belongs to an account"
                );
                self.accounts
                    .get_mut(id)
                    .ok_or_else(|| Refusal::NotFound("no such account".to_owned()))?
                    .keys
                    .insert(key.clone());
            }
            DirectoryEntry::AccountKeyRemoved { id, key } => {
                let account = self
                    .accounts
                    .get_mut(id)
                    .ok_or_else(|| Refusal::NotFound("no such account".to_owned()))?;
                refuse_unless!(
                    account.keys.remove(key),
                    NotFound,
                    "that key is not on the account"
                );
            }
            DirectoryEntry::AccountEmailConfirmed { id, email } => {
                let holder = self.account_for_email(email);
                refuse_unless!(
                    holder.is_none() || holder.as_deref() == Some(id.as_str()),
                    Conflict,
                    "that address already belongs to an account"
                );
                self.accounts
                    .get_mut(id)
                    .ok_or_else(|| Refusal::NotFound("no such account".to_owned()))?
                    .emails
                    .insert(email.clone());
            }
            DirectoryEntry::AccountEmailRemoved { id, email } => {
                let account = self
                    .accounts
                    .get_mut(id)
                    .ok_or_else(|| Refusal::NotFound("no such account".to_owned()))?;
                refuse_unless!(
                    account.emails.remove(email),
                    NotFound,
                    "that address is not on the account"
                );
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
                        name: clean_name_mode(name, refuse_format)?,
                        description: clean_description_mode(description, refuse_format)?,
                        scopes: permissions(*scopes)?,
                        public: false,
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
                let name = clean_name_mode(name, refuse_format)?;
                let description = clean_description_mode(description, refuse_format)?;
                let scopes = permissions(*scopes)?;
                let group = self.groups.get_mut(id).context("no such group")?;
                group.name = name;
                group.description = description;
                group.scopes = scopes;
            }
            DirectoryEntry::GroupVisibility { id, public } => {
                self.groups.get_mut(id).context("no such group")?.public = *public;
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
            DirectoryEntry::FriendRequested { from, to, id } => {
                ensure_account_id(from)?;
                ensure_account_id(to)?;
                ensure!(from != to, "no one befriends themselves");
                refuse_unless!(!self.are_friends(from, to), Conflict, "already friends");
                refuse_unless!(
                    !self.requests.contains_key(&(from.clone(), to.clone()))
                        && !self.requests.contains_key(&(to.clone(), from.clone())),
                    Conflict,
                    "a request is already pending"
                );
                if let Some(id) = id {
                    refuse_unless!(
                        !self.removed_requests.contains(id),
                        Conflict,
                        "request {id} was voided by a friend removal"
                    );
                }
                self.note_envelope(id.as_deref())?;
                self.requests.insert((from.clone(), to.clone()), id.clone());
            }
            DirectoryEntry::FriendAccepted {
                from,
                to,
                request,
                id,
            } => {
                let Some(pending) = self.requests.get(&(from.clone(), to.clone())) else {
                    refuse!(Conflict, "no pending request from {from} to {to}");
                };
                refuse_unless!(
                    pending == request,
                    Conflict,
                    "no pending request {} from {from} to {to}",
                    request.as_deref().unwrap_or("without an ID")
                );
                self.note_envelope(id.as_deref())?;
                self.requests.remove(&(from.clone(), to.clone()));
                self.friends.insert(pair(from, to));
                if let Some(request) = request {
                    self.friend_requests.insert(pair(from, to), request.clone());
                }
            }
            DirectoryEntry::FriendRemoved { a, b, requests, .. } => {
                let requests = self.pair_bound_requests(a, b, requests);
                let was_friends = self.friends.remove(&pair(a, b));
                ensure!(was_friends || !requests.is_empty(), "not friends");
                self.friend_requests.remove(&pair(a, b));
                self.requests
                    .retain(|(from, to), _| pair(from, to) != pair(a, b));
                self.removed_requests.extend(requests.iter().cloned());
            }
            DirectoryEntry::FriendRemovalNoted { .. } => {
                // A record that a removal artifact was stored without
                // an effect (#5624); it changes no directory state.
                // Replay collects the artifact hash from the journal
                // line into `noted_removal_artifacts`.
            }
        }
        Ok(())
    }

    fn friendship_requests(&self, a: &str, b: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .friend_requests
            .get(&pair(a, b))
            .cloned()
            .into_iter()
            .collect();
        ids.extend(self.pending_id(a, b));
        ids.extend(self.pending_id(b, a));
        ids.sort();
        ids.dedup();
        debug_assert!(ids.len() <= artifact::MAX_FRIEND_REMOVE_REQUESTS);
        ids
    }

    /// The pair's stored request IDs that are noncanonical — persisted
    /// before friend-request admission validation (#5624). A signed
    /// removal artifact cannot name them (the #3146 receipt contract
    /// refuses noncanonical IDs), so receipt and recovery pair an
    /// empty-request removal artifact with these local IDs to void
    /// exactly the legacy state it ends.
    fn legacy_pair_requests(&self, a: &str, b: &str) -> Vec<String> {
        self.friendship_requests(a, b)
            .into_iter()
            .filter(|id| !artifact::is_friend_request_id(id))
            .collect()
    }

    /// Consume the journal entry a stored removal artifact was
    /// applied with, if replay saw one (#5624). `store_artifact`
    /// appends the artifact first and the journal entry second, so an
    /// artifact with no entry was stored by a removal that crashed
    /// before journaling — recovery must still apply it — while an
    /// artifact with an entry was already applied and may only be
    /// re-applied where it still names current state.
    ///
    /// Entries journaled by current code carry the hash of the
    /// artifact they were applied with, and correspondence is by that
    /// hash alone. A local removal (`remove_friend_as`) journals an
    /// entry carrying no artifact hash, and an artifact must never
    /// consume it: however similar their requests look, the entry
    /// belongs to a different removal than the artifact. Entries
    /// journaled before hashes were recorded carry no hash either, so
    /// for them correspondence falls back to the request sets: the
    /// producer's entry carries the pair's stored IDs verbatim, so it
    /// may hold noncanonical IDs the artifact cannot name (#3146); a
    /// receiver's entry holds the pair-bound subset of the artifact's
    /// IDs. An empty artifact does not correspond to a hashless entry
    /// by pair alone — a hashless empty entry is also exactly what a
    /// local removal of an ID-less friendship journals, so pair-only
    /// matching would let an interrupted removal consume a local
    /// removal's entry and resurrect the friendship it targeted.
    /// Entries are consumed one per artifact.
    fn take_journaled_removal(
        &mut self,
        a: &str,
        b: &str,
        artifact_requests: &[String],
        artifact_hash: &str,
    ) -> bool {
        if let Some(index) = self
            .journaled_removals
            .iter()
            .position(|(_, _, _, entry_artifact)| entry_artifact.as_deref() == Some(artifact_hash))
        {
            self.journaled_removals.remove(index);
            return true;
        }
        if artifact_requests.is_empty() {
            return false;
        }
        use std::collections::BTreeSet;
        let target = pair(a, b);
        let artifact_set: BTreeSet<&str> = artifact_requests.iter().map(String::as_str).collect();
        let position =
            self.journaled_removals
                .iter()
                .position(|(ja, jb, jrequests, entry_artifact)| {
                    if entry_artifact.is_some() || pair(ja, jb) != target {
                        return false;
                    }
                    let journal_set: BTreeSet<&str> =
                        jrequests.iter().map(String::as_str).collect();
                    journal_set == artifact_set
                        || journal_set.is_subset(&artifact_set)
                        || (artifact_set.is_subset(&journal_set)
                            && journal_set
                                .difference(&artifact_set)
                                .all(|id| !artifact::is_friend_request_id(id)))
                });
        match position {
            Some(index) => {
                self.journaled_removals.remove(index);
                true
            }
            None => false,
        }
    }

    /// The journal entry a friend-removal artifact should produce
    /// against the current state, if any (#5624). Recovery scans every
    /// stored artifact, including ones already journaled, so a removal
    /// must only end the state it actually targeted:
    ///
    /// - a friendship is ended when the artifact names its formed-by
    ///   request, or names no requests at all — the legacy signature,
    ///   since a removal of a legacy-formed friendship cannot legally
    ///   name its noncanonical formed-by ID (#3146);
    /// - a removal naming requests that do not include the current
    ///   formed-by ID targeted an earlier formation of the pair and
    ///   leaves a later friendship (legacy or canonical) standing;
    /// - an artifact with no journal entry is an interrupted removal
    ///   (see [`Self::take_journaled_removal`]): the state it targeted
    ///   is the current state, so an empty artifact also ends a
    ///   friendship formed with no request ID at all, which a journaled
    ///   empty artifact must leave standing (it cannot be told apart
    ///   from a newer ID-less friendship the removal predates);
    /// - with no friendship, pair-bound named requests cancel pending
    ///   requests as before, and an empty-request artifact cancels the
    ///   pair's locally held legacy pending IDs.
    fn friend_removal_effect(
        &self,
        a: &str,
        b: &str,
        artifact_requests: &[String],
        journaled: bool,
        artifact_hash: &str,
    ) -> Option<DirectoryEntry> {
        let bound = self.pair_bound_requests(a, b, artifact_requests);
        if self.are_friends(a, b) {
            let formed_by = self.friend_requests.get(&pair(a, b));
            if artifact_requests.is_empty() {
                // The legacy signature ends only a friendship it could
                // have targeted: one formed by a noncanonical request
                // ID (#5624). A friendship formed by a canonical request
                // is not its target (its removal artifact would name
                // the ID). A friendship formed with no request ID at
                // all (the local request_friend_as flow) is a target
                // only when the artifact is unjournaled — an interrupted
                // removal of exactly this friendship; a journaled empty
                // artifact predates any newer ID-less friendship and
                // leaves it standing.
                let legacy_formed = formed_by.is_some_and(|id| !artifact::is_friend_request_id(id));
                let interrupted_id_less = !journaled && formed_by.is_none();
                if !legacy_formed && !interrupted_id_less {
                    return None;
                }
                return Some(DirectoryEntry::FriendRemoved {
                    a: a.to_owned(),
                    b: b.to_owned(),
                    requests: self.friendship_requests(a, b),
                    artifact: Some(artifact_hash.to_owned()),
                });
            }
            if !formed_by.is_some_and(|id| artifact_requests.contains(id)) {
                return None;
            }
            return Some(DirectoryEntry::FriendRemoved {
                a: a.to_owned(),
                b: b.to_owned(),
                requests: bound,
                artifact: Some(artifact_hash.to_owned()),
            });
        }
        if !bound.is_empty() {
            // Applying a removal clears every pending request for the
            // pair, so a stored removal may only be (re)applied against
            // pending state it actually names. A pair holding a newer
            // pending request the artifact does not name keeps it; the
            // named IDs were already voided by the removal's own
            // journal entry when it first applied.
            let has_pending = self.requests.contains_key(&(a.to_owned(), b.to_owned()))
                || self.requests.contains_key(&(b.to_owned(), a.to_owned()));
            let pending: Vec<String> = [self.pending_id(a, b), self.pending_id(b, a)]
                .into_iter()
                .flatten()
                .collect();
            // A pending entry with no envelope ID (request_friend_as)
            // is pending state too: pending_id flattens it away, but an
            // artifact naming only other IDs did not target it.
            if has_pending && !pending.iter().any(|id| bound.contains(id)) {
                return None;
            }
            return Some(DirectoryEntry::FriendRemoved {
                a: a.to_owned(),
                b: b.to_owned(),
                requests: bound,
                artifact: Some(artifact_hash.to_owned()),
            });
        }
        if artifact_requests.is_empty() {
            let legacy = self.legacy_pair_requests(a, b);
            if !legacy.is_empty() {
                return Some(DirectoryEntry::FriendRemoved {
                    a: a.to_owned(),
                    b: b.to_owned(),
                    requests: legacy,
                    artifact: Some(artifact_hash.to_owned()),
                });
            }
        }
        None
    }

    /// Bind a removal's request IDs to the named pair (#4849): drop any ID
    /// this server already knows belongs to another pair's pending or
    /// formed-by request. Unknown IDs are kept so a legitimate
    /// never-delivered removal still voids its own future delivery; a pair
    /// with no friendship and no surviving IDs of its own is stored alone.
    fn pair_bound_requests(&self, a: &str, b: &str, requests: &[String]) -> Vec<String> {
        use std::collections::BTreeSet;
        let target = pair(a, b);
        let mut foreign: BTreeSet<&str> = BTreeSet::new();
        for ((from, to), id) in &self.requests {
            if pair(from, to) != target {
                if let Some(id) = id {
                    foreign.insert(id.as_str());
                }
            }
        }
        for (known_pair, id) in &self.friend_requests {
            if *known_pair != target {
                foreign.insert(id.as_str());
            }
        }
        requests
            .iter()
            .filter(|id| !foreign.contains(id.as_str()))
            .cloned()
            .collect()
    }

    /// Whether two accounts are friends on this server.
    #[must_use]
    pub fn are_friends(&self, a: &str, b: &str) -> bool {
        self.friends.contains(&pair(a, b))
    }

    fn note_envelope(&mut self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            ensure!(
                self.seen.insert(id.to_owned()),
                "envelope {id} was already applied"
            );
        }
        Ok(())
    }

    /// Whether `target` is `from` or is nested, at any depth, inside it.
    fn reaches(&self, from: u64, target: u64) -> bool {
        self.reaches_visited(from, target, &mut BTreeSet::new())
    }

    fn reaches_visited(&self, from: u64, target: u64, visited: &mut BTreeSet<u64>) -> bool {
        if from == target {
            return true;
        }
        if !visited.insert(from) {
            return false;
        }
        self.members
            .range((from, Member::Group { id: 0 })..=(from, Member::Group { id: u64::MAX }))
            .any(|((_, member), _)| match member {
                Member::Group { id } => self.reaches_visited(*id, target, visited),
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

fn artifact_path(journal: &Path) -> PathBuf {
    journal.with_file_name(artifact::FILE)
}

fn signing_key(secret: &[u8; 32]) -> SigningKey {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update(secret);
    SigningKey::from_bytes(&hasher.finalize().into())
}

fn may_act_for(actor: Actor<'_>, account: &str) -> bool {
    match actor {
        Actor::Server => true,
        Actor::Account(me) => me == account,
    }
}

/// A public key as lower-case hex, if it is an Ed25519 public key.
///
/// # Errors
///
/// Fails if the text is not 64 hex characters or not a valid key.
pub fn normalize_key(key: &str) -> Result<String> {
    let key = key.trim().to_ascii_lowercase();
    let bytes = hex_to_array::<32>(&key).context("a public key is 64 hex characters")?;
    VerifyingKey::from_bytes(&bytes).context("not an Ed25519 public key")?;
    Ok(key)
}

/// An email address in lower case, checked for shape but not deliverability.
///
/// # Errors
///
/// Fails if the text does not look like `local@domain.tld`.
pub fn normalize_email(address: &str) -> Result<String> {
    let address = address.trim().to_ascii_lowercase();
    ensure!(address.len() <= 254, "email address is too long");
    let (local, domain) = address
        .split_once('@')
        .context("an email address has one @")?;
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c));
    let domain_ok = domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    ensure!(local_ok && domain_ok, "not an email address");
    Ok(address)
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
    clean_name_mode(name, true)
}

fn clean_name_mode(name: &str, refuse_format: bool) -> Result<String> {
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
    if refuse_format {
        ensure!(
            !name.chars().any(crate::petname::is_format_char),
            "name must not contain format characters"
        );
    }
    Ok(name.to_owned())
}

fn clean_description(description: &str) -> Result<String> {
    clean_description_mode(description, true)
}

fn clean_description_mode(description: &str, refuse_format: bool) -> Result<String> {
    let description = description.trim();
    ensure!(
        description.chars().count() <= MAX_DESCRIPTION_CHARS,
        "description must be at most {MAX_DESCRIPTION_CHARS} characters"
    );
    ensure!(
        !description.chars().any(|c| c.is_control() && c != '\n'),
        "description must not contain control characters"
    );
    if refuse_format {
        ensure!(
            !description.chars().any(crate::petname::is_format_char),
            "description must not contain format characters"
        );
    }
    Ok(description.to_owned())
}

pub fn hex_to_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    // Every byte must be an ASCII hex digit (#2517): u8::from_str_radix
    // also accepts a leading '+', so without this gate a byte 0x00-0x0f
    // has two accepted spellings ('0x' and '+x') and key strings stop
    // being canonical.
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0_u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::test_forgeries::{IDENTITY_KEY, identity_forgery, with_non_canonical_s};

    fn all() -> WorldScopes {
        WorldScopes::GUEST.union(WorldScopes::DELEGATE)
    }

    fn account(directory: &mut Directory, name: &str) -> (String, String) {
        let (credential, _) = directory.sign_up(name).unwrap();
        let id = directory.account_for(&credential).unwrap();
        (id, credential)
    }

    fn diamond_directory() -> (Directory, String, Vec<Vec<u64>>, u64) {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let (leaf, _) = directory.create_group(&ann, "Leaf", "", all()).unwrap();
        let (holder, _) = directory.create_group(&ann, "Holder", "", all()).unwrap();
        let mut layers = vec![vec![leaf]];

        for layer in 0..25 {
            let children = layers.last().unwrap();
            let mut parents = Vec::new();
            for branch in 0..2 {
                let (parent, _) = directory
                    .create_group(&ann, &format!("Layer {layer} branch {branch}"), "", all())
                    .unwrap();
                for child in children {
                    let held_at = if layer == 0 {
                        if branch == 0 {
                            Level::Read
                        } else {
                            Level::Manage
                        }
                    } else {
                        Level::Write
                    };
                    directory
                        .members
                        .insert((parent, Member::Group { id: *child }), held_at);
                }
                parents.push(parent);
            }
            layers.push(parents);
        }
        directory
            .members
            .insert((leaf, Member::Account { id: bo.clone() }), Level::Admin);
        (directory, bo, layers, holder)
    }

    #[test]
    fn replica_facts_create_nested_groups_before_memberships_and_replay_old_names() {
        let mut writer = Directory::new();
        let (ann, ann_credential) = account(&mut writer, "Ann");
        let (bo, bo_credential) = account(&mut writer, "Bo");
        writer
            .request_friend(&ann_credential, &bo, "replica-facts-test", 1)
            .unwrap();
        writer
            .accept_friend(&bo_credential, &ann, "replica-facts-test", 2)
            .unwrap();
        let (outer, _) = writer
            .create_group(&ann, "Outer", "nested", WorldScopes::JOIN)
            .unwrap();
        let (inner, _) = writer
            .create_group(&ann, "Inner", "nested", WorldScopes::DELEGATE)
            .unwrap();
        writer
            .set_member(
                Actor::Server,
                outer,
                Member::Group { id: inner },
                Some(Level::Admin),
            )
            .unwrap();
        // The outer group now has no direct account admin; its admin is
        // inherited through the nested group, which `ensure_admins` allows.
        writer
            .set_member(
                Actor::Server,
                outer,
                Member::Account { id: ann.clone() },
                None,
            )
            .unwrap();
        writer
            .set_member(
                Actor::Server,
                inner,
                Member::Account { id: bo.clone() },
                Some(Level::Read),
            )
            .unwrap();

        // Historical directories may contain a format character accepted
        // before #1778. Replay admits it; a replica projection must preserve
        // it rather than passing it through the stricter new-write check.
        writer
            .apply_replay(&DirectoryEntry::GroupUpdated {
                id: inner,
                name: "Inner\u{200B}".to_owned(),
                description: "nested".to_owned(),
                scopes: WorldScopes::DELEGATE.bits(),
            })
            .unwrap();
        let subjects = BTreeSet::from([bo.clone()]);
        let facts = writer.replica_facts_for(&subjects).unwrap();
        let group_creates: Vec<u64> = facts
            .iter()
            .filter_map(|entry| match entry {
                DirectoryEntry::GroupCreated { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(group_creates, [outer, inner]);
        let mut replica = Directory::new();
        for fact in &facts {
            replica.apply_replica_fact(fact).unwrap();
        }
        assert_eq!(replica.ceiling(&bo), writer.ceiling(&bo));
        assert_eq!(replica.group(inner).unwrap().name, "Inner\u{200B}");
        assert!(replica.are_friends(&ann, &bo));

        // Preserve the friend-request ID in projected facts so a signed
        // FriendRemove artifact can invalidate the projected relationship.
        let removal = writer.remove_friend(&ann_credential, &bo).unwrap();
        let dir = TempDir::new().unwrap();
        let journal = dir.path().join("directory.jsonl");
        let mut text = String::new();
        for fact in &facts {
            text.push_str(&serde_json::to_string(fact).unwrap());
            text.push('\n');
        }
        std::fs::write(&journal, text).unwrap();
        artifact::append(&artifact_path(&journal), &removal).unwrap();
        let replayed = Directory::replay_read_only(&journal).unwrap();
        assert!(!replayed.are_friends(&ann, &bo));
    }

    #[test]
    fn replica_facts_restore_founder_with_direct_read_and_inherited_admin() {
        // Codex P2 on PR #4708: a founder with direct `Read` membership
        // who inherits `Admin` through a nested group could not be
        // restored — the projection emitted the founder's `MemberSet`
        // downgrade before the nested membership, so replay failed
        // `ensure_admins` and replica startup failed.
        let mut writer = Directory::new();
        let (ann, _) = account(&mut writer, "Ann");
        let (outer, _) = writer
            .create_group(&ann, "Outer", "nested", WorldScopes::JOIN)
            .unwrap();
        let (inner, _) = writer
            .create_group(&ann, "Inner", "nested", WorldScopes::DELEGATE)
            .unwrap();
        writer
            .set_member(
                Actor::Server,
                outer,
                Member::Group { id: inner },
                Some(Level::Admin),
            )
            .unwrap();
        writer
            .set_member(
                Actor::Server,
                outer,
                Member::Account { id: ann.clone() },
                Some(Level::Read),
            )
            .unwrap();
        // Ann's effective level in `outer` is Admin, inherited via her
        // founder admin of `inner`; her direct level there is only Read.
        assert_eq!(writer.level_in(&ann, outer), Some(Level::Admin));
        let subjects = BTreeSet::from([ann.clone()]);
        let facts = writer.replica_facts_for(&subjects).unwrap();
        let mut replica = Directory::new();
        for fact in &facts {
            replica.apply_replica_fact(fact).unwrap();
        }
        assert_eq!(replica.level_in(&ann, outer), Some(Level::Admin));
        assert_eq!(replica.ceiling(&ann), writer.ceiling(&ann));
    }

    #[test]
    fn read_only_directory_open_validates_artifacts_without_a_directory_journal() {
        let dir = TempDir::new().unwrap();
        let journal = dir.path().join("directory.jsonl");
        std::fs::write(artifact_path(&journal), b"not-an-artifact\n").unwrap();
        assert!(Directory::replay_read_only(&journal).is_err());
    }

    #[test]
    fn read_only_directory_open_decodes_only_complete_utf8_lines() {
        let dir = TempDir::new().unwrap();
        let journal = dir.path().join("directory.jsonl");
        let entry = DirectoryEntry::AccountKeyed {
            id: "01".repeat(32),
            name: "Alice".to_owned(),
        };
        let mut bytes = serde_json::to_vec(&entry).unwrap();
        bytes.push(b'\n');
        bytes.extend_from_slice(&[0xe2, 0x80]);
        std::fs::write(&journal, &bytes).unwrap();
        let replayed = Directory::replay_read_only(&journal).unwrap();
        assert_eq!(replayed.accounts().count(), 1);
        assert_eq!(std::fs::read(&journal).unwrap(), bytes);
    }

    #[test]
    fn test_cleaners_reject_format_characters_and_accept_plain_text() {
        assert!(clean_name("Al\u{200B}ice").is_err());
        assert!(clean_name("Alice\u{202E}Admin").is_err());
        assert!(clean_description("Group\u{202E} description").is_err());

        assert_eq!(clean_name("Alice").unwrap(), "Alice");
        assert_eq!(
            clean_description("Group description\nwith a newline").unwrap(),
            "Group description\nwith a newline"
        );
    }

    #[test]
    fn replay_tolerates_pre_fix_format_characters_in_group_fields() {
        // #1778 Codex P1: a journal written before the Cf refusal can
        // legitimately contain format characters in a group name or
        // description. Replay must reopen such a world, while new
        // writes through the public path still refuse Cf.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::new();
        let (ann, _) = account_into(&mut directory, &path, "Ann");
        let (group, entry) = directory
            .create_group(&ann, "Crew", "hello", all())
            .unwrap();
        Directory::append(&path, &entry).unwrap();

        let historical = DirectoryEntry::GroupUpdated {
            id: group,
            name: "Crew\u{202E}X".to_owned(),
            description: "hi\u{200B} there".to_owned(),
            scopes: all().bits(),
        };
        let mut line = serde_json::to_string(&historical).unwrap();
        line.push('\n');
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&line);
        std::fs::write(&path, text).unwrap();

        let replayed = Directory::replay(&path).unwrap();
        assert_eq!(replayed.group(group).unwrap().name, "Crew\u{202E}X");
        assert_eq!(
            replayed.group(group).unwrap().description,
            "hi\u{200B} there"
        );

        assert!(
            directory
                .create_group(&ann, "Bad\u{200B}Name", "hello", all())
                .is_err(),
            "new writes must still refuse format characters"
        );
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

    fn home_server(seed: u8) -> HomeServer {
        HomeServer {
            endpoint: iroh_base::SecretKey::from_bytes(&[seed; 32])
                .public()
                .to_string(),
            world_id: "home".to_owned(),
        }
    }

    fn friends_across_servers(
        dir: Option<&Path>,
    ) -> (Directory, Directory, String, String, String) {
        let mut home = match dir {
            Some(path) => Directory::open(path).unwrap(),
            None => Directory::new(),
        };
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        home.receive(&accept, "home", 2).unwrap();
        (home, away, ann, bo, ann_credential)
    }

    #[test]
    fn a_removal_ends_the_friendship_on_both_servers_and_voids_a_request_it_never_delivered() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        let mut late = away.clone();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        home.receive(&accept, "home", 2).unwrap();
        let removal = home.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!home.are_friends(&ann, &bo));

        let mut tampered = removal.clone();
        tampered.body["requests"] = json!([]);
        assert!(away.receive_artifact(&tampered).is_err());

        assert_eq!(away.receive_artifact(&removal).unwrap(), Stored::New);
        assert!(!away.are_friends(&ann, &bo));
        assert_eq!(away.receive_artifact(&removal).unwrap(), Stored::Duplicate);

        assert_eq!(late.receive_artifact(&removal).unwrap(), Stored::New);
        let stale = late.receive(&request, "away", 1).unwrap_err();
        assert!(
            matches!(stale.downcast_ref::<Refusal>(), Some(Refusal::Conflict(_))),
            "{stale:#}"
        );
    }

    #[test]
    fn a_friend_request_with_a_noncanonical_id_is_refused_and_never_persisted() {
        // #5624 regression: the oracle is the canonical shape the producer
        // mints - 16 bytes as 32 lowercase hex (`is_friend_request_id`).
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, _bo_credential) = account(&mut away, "Bo");
        // Craft a correctly signed friend request whose ID is "r1": start
        // from a genuine envelope, swap the ID, and re-sign the body, so
        // only the ID shape is wrong.
        let (mut envelope, _) = home
            .prepare_request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        envelope.id = "r1".to_owned();
        let key = key_of(&ann_credential);
        let body = serde_json::to_vec(&envelope.body()).unwrap();
        envelope.signature = hex_encode(&key.sign(&body).to_bytes());
        assert!(envelope.verify().is_ok());
        let refused = away.receive(&envelope, "away", 1).unwrap_err();
        assert!(
            matches!(
                refused.downcast_ref::<Refusal>(),
                Some(Refusal::Forbidden(_))
            ),
            "{refused:#}"
        );
        // Nothing was persisted: no pending request is held, and the same
        // pair can still go through the genuine flow afterwards.
        assert!(away.requests.is_empty());
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        assert_eq!(
            away.pending_id(&ann, &bo).as_deref(),
            Some(request.id.as_str())
        );
    }

    #[test]
    fn a_friendship_formed_by_a_legacy_noncanonical_request_can_be_removed() {
        // #5624 regression: state persisted before admission validation -
        // a request whose ID is "r1" was applied directly (the path legacy
        // journal entries take) and accepted, forming the friendship.
        let mut directory = Directory::new();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r1".to_owned()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some("r1".to_owned()),
                id: None,
            })
            .unwrap();
        assert!(directory.are_friends(&ann, &bo));

        // Before the fix this failed: the signed removal named "r1" and
        // the artifact receipt contract (#3146) refused it on insert,
        // stranding the friendship.
        let removal = directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        // The stored ID was carried verbatim into the local removal entry
        // and voided.
        assert!(directory.removed_requests.contains("r1"));
        // The signed artifact stays inside the #3146 contract: it parses
        // strictly and names no noncanonical ID, so receiving servers can
        // still apply it and end the friendship by pair.
        let Some(Kind::FriendRemove(body)) = removal.parsed().unwrap() else {
            panic!("removal artifact did not parse as a friend removal");
        };
        assert!(
            body.requests
                .iter()
                .all(|id| artifact::is_friend_request_id(id))
        );

        // A pending (never accepted) legacy request is removable too.
        let (cy, cy_credential) = account(&mut directory, "Cy");
        let (di, _di_credential) = account(&mut directory, "Di");
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: cy.clone(),
                to: di.clone(),
                id: Some("r2".to_owned()),
            })
            .unwrap();
        directory.remove_friend(&cy_credential, &di).unwrap();
        assert!(directory.removed_requests.contains("r2"));
        assert!(directory.pending_id(&cy, &di).is_none());
    }

    #[test]
    fn a_legacy_removal_takes_effect_on_read_only_replay_when_its_journal_entry_is_lost() {
        // #5624 (Codex P2 on the fix PR): the producer's removal artifact
        // for a legacy-formed friendship names no request ID, and
        // read-only replay only applied removals naming the formed-by ID,
        // so a replica kept the friendship. Seed legacy state in a
        // journaled directory, remove, then lose the removal's journal
        // line (crash between storing the artifact and journaling).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r1".to_owned()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some("r1".to_owned()),
                id: None,
            })
            .unwrap();
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        drop(directory);

        let text = std::fs::read_to_string(&path).unwrap();
        let kept: String = text
            .lines()
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, kept).unwrap();

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(!replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(!reopened.are_friends(&ann, &bo));
    }

    #[test]
    fn an_interrupted_removal_of_an_id_less_friendship_is_recovered_on_reopen() {
        // #5624 (Codex P2 on the fix PR): a friendship formed through
        // the local request_friend_as flow has no formed-by ID, and its
        // signed removal artifact names no requests. If the process
        // crashes after store_artifact appends the artifact but before
        // it appends the journal entry, recovery must not resurrect
        // the friendship: the artifact has no journaled counterpart,
        // which is what tells it apart from an already-journaled
        // historical removal that a newer ID-less friendship must
        // survive.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory.request_friend_as(&ann, &bo).unwrap();
        directory.accept_friend_as(&bo, &ann).unwrap();
        assert!(directory.are_friends(&ann, &bo));
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        drop(directory);

        // Lose the removal's journal line (the crash window in
        // store_artifact: artifact appended, journal entry not).
        let text = std::fs::read_to_string(&path).unwrap();
        let kept: String = text
            .lines()
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, kept).unwrap();

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(!replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(!reopened.are_friends(&ann, &bo));
    }

    #[test]
    fn a_stored_no_effect_removal_does_not_end_a_later_id_less_friendship_on_reopen() {
        // #5624 (Codex P2 on the fix PR): a removal artifact received
        // for a pair this server never befriended is stored without an
        // effect. If the pair later forms an ID-less friendship, the
        // stored artifact must not be mistaken for an interrupted
        // removal of it — receipt journals a FriendRemovalNoted entry
        // for exactly this distinction.
        let mut producer = Directory::new();
        let (ann, ann_credential) = account(&mut producer, "Ann");
        let (bo, _bo_credential) = account(&mut producer, "Bo");
        producer.request_friend_as(&ann, &bo).unwrap();
        producer.accept_friend_as(&bo, &ann).unwrap();
        let removal = producer.remove_friend(&ann_credential, &bo).unwrap();
        assert!(
            matches!(removal.parsed().unwrap(), Some(Kind::FriendRemove(r)) if r.requests.is_empty())
        );

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut receiver = Directory::open(&path).unwrap();
        assert_eq!(receiver.receive_artifact(&removal).unwrap(), Stored::New);
        receiver.request_friend_as(&ann, &bo).unwrap();
        receiver.accept_friend_as(&bo, &ann).unwrap();
        assert!(receiver.are_friends(&ann, &bo));
        drop(receiver);

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(reopened.are_friends(&ann, &bo));
    }

    #[test]
    fn an_interrupted_removal_is_not_defeated_by_an_earlier_local_removal_on_reopen() {
        // #5624 (Codex P2 on the fix PR): remove_friend_as journals a
        // removal entry but stores no artifact. When the pair later
        // re-forms an ID-less friendship and a signed removal of it is
        // interrupted (artifact stored, journal entry lost), recovery
        // must not let the artifact consume the earlier local entry:
        // correspondence is by the hash of the artifact an entry was
        // journaled with, and the local entry carries none.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory.request_friend_as(&ann, &bo).unwrap();
        directory.accept_friend_as(&bo, &ann).unwrap();
        directory.remove_friend_as(&ann, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        directory.request_friend_as(&ann, &bo).unwrap();
        directory.accept_friend_as(&bo, &ann).unwrap();
        assert!(directory.are_friends(&ann, &bo));
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        drop(directory);

        // Lose the signed removal's journal line (the crash window in
        // store_artifact: artifact appended, journal entry not). The
        // earlier local removal's entry is now the only removal entry
        // left in the journal.
        let text = std::fs::read_to_string(&path).unwrap();
        let kept: String = text
            .lines()
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, kept).unwrap();

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(!replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(!reopened.are_friends(&ann, &bo));
    }

    /// Property coverage for #5624, per the repo testing doctrine
    /// (AGENTS.md: state machines, replication, and ID formats get
    /// property tests with a named oracle; examples do not replace
    /// them). Oracles: `artifact::is_friend_request_id` for the
    /// admission shape, and the live directory itself for recovery —
    /// reopening and read-only replay must reproduce the exact state
    /// the journal and artifact log were written from.
    mod property_tests {
        use super::*;
        use ed25519_dalek::Signer as _;
        use proptest::prelude::*;

        fn canonical_id(n: u32) -> String {
            format!(
                "{:08x}{:08x}{:08x}{:08x}",
                n,
                n.wrapping_mul(0x9e37_79b9),
                n ^ 0xa5a5_a5a5,
                !n
            )
        }

        fn id_strategy() -> impl Strategy<Value = String> {
            prop_oneof![
                any::<u32>().prop_map(canonical_id),
                any::<u32>().prop_map(|n| format!("r{}", n % 1000)),
                any::<u32>().prop_map(|n| canonical_id(n).to_uppercase()),
                any::<u32>().prop_map(|n| canonical_id(n)[..31].to_owned()),
                any::<u32>().prop_map(|n| format!("{}a", canonical_id(n))),
                any::<u32>().prop_map(|n| format!("g{}", &canonical_id(n)[1..])),
            ]
        }

        fn strip_last_journal_line(path: &std::path::Path) {
            let text = std::fs::read_to_string(path).unwrap();
            let kept: String = text
                .lines()
                .rev()
                .skip(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|line| format!("{line}\n"))
                .collect();
            std::fs::write(path, kept).unwrap();
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(48))]

            #[test]
            fn admission_accepts_exactly_the_canonical_id_shape(id in id_strategy()) {
                let mut home = Directory::new();
                let mut away = Directory::new();
                let (ann, ann_credential) = account(&mut home, "Ann");
                let (bo, _bo_credential) = account(&mut away, "Bo");
                let (mut envelope, _) = home
                    .prepare_request_friend(&ann_credential, &bo, "away", 1)
                    .unwrap();
                envelope.id = id.clone();
                let key = key_of(&ann_credential);
                let body = serde_json::to_vec(&envelope.body()).unwrap();
                envelope.signature = hex_encode(&key.sign(&body).to_bytes());
                prop_assert!(envelope.verify().is_ok());
                let received = away.receive(&envelope, "away", 1);
                if artifact::is_friend_request_id(&id) {
                    prop_assert!(received.is_ok(), "canonical id {id} refused");
                    let pending = away.pending_id(&ann, &bo);
                    prop_assert_eq!(pending.as_deref(), Some(id.as_str()));
                } else {
                    prop_assert!(received.is_err(), "noncanonical id {id} admitted");
                    prop_assert!(
                        away.requests.is_empty(),
                        "noncanonical id {id} persisted state"
                    );
                }
            }

            #[test]
            fn a_durable_removal_is_recovered_after_its_journal_entry_is_lost(
                kind in 0u8..3,
                n in any::<u32>(),
            ) {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("directory.jsonl");
                let mut directory = Directory::open(&path).unwrap();
                let (ann, ann_credential) = account(&mut directory, "Ann");
                let (bo, _bo_credential) = account(&mut directory, "Bo");
                let id: Option<String> = match kind {
                    0 => Some(canonical_id(n)),
                    1 => Some(format!("r{}", n % 1000)),
                    _ => None,
                };
                directory
                    .apply(&DirectoryEntry::FriendRequested {
                        from: ann.clone(),
                        to: bo.clone(),
                        id: id.clone(),
                    })
                    .unwrap();
                directory
                    .apply(&DirectoryEntry::FriendAccepted {
                        from: ann.clone(),
                        to: bo.clone(),
                        request: id.clone(),
                        id: None,
                    })
                    .unwrap();
                prop_assert!(directory.are_friends(&ann, &bo));
                directory.remove_friend(&ann_credential, &bo).unwrap();
                drop(directory);
                strip_last_journal_line(&path);

                let replayed = Directory::replay_read_only(&path).unwrap();
                prop_assert!(!replayed.are_friends(&ann, &bo));
                if let Some(id) = &id {
                    prop_assert!(replayed.removed_requests.contains(id));
                }
                drop(replayed);

                let reopened = Directory::open(&path).unwrap();
                prop_assert!(!reopened.are_friends(&ann, &bo));
            }

            #[test]
            fn reopen_and_read_only_replay_reproduce_live_state(
                ops in prop::collection::vec(
                    (0u8..5, 0u8..2, 0u8..3, any::<u32>()),
                    0..14,
                ),
            ) {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("directory.jsonl");
                let mut directory = Directory::open(&path).unwrap();
                let (ann, ann_credential) = account(&mut directory, "Ann");
                let (bo, _bo_credential) = account(&mut directory, "Bo");
                let (cy, _cy_credential) = account(&mut directory, "Cy");
                let others = [bo.clone(), cy.clone()];
                for (op, pair_idx, kind, n) in ops {
                    let other = &others[pair_idx as usize];
                    // Canonical and ID-less formations only: successive
                    // legacy formations are the one sequence the
                    // journal cannot order (see the trade-off recorded
                    // on PR #6239), so they are covered by the targeted
                    // examples above instead of this invariant.
                    let generated = match kind {
                        0 => Some(canonical_id(n)),
                        1 => None,
                        _ => Some(canonical_id(n.wrapping_add(1))),
                    };
                    match op {
                        0 => {
                            let _ = directory.apply(&DirectoryEntry::FriendRequested {
                                from: ann.clone(),
                                to: other.clone(),
                                id: generated.clone(),
                            });
                            let _ = directory.apply(&DirectoryEntry::FriendAccepted {
                                from: ann.clone(),
                                to: other.clone(),
                                request: generated,
                                id: None,
                            });
                        }
                        1 => {
                            let _ = directory.apply(&DirectoryEntry::FriendRequested {
                                from: ann.clone(),
                                to: other.clone(),
                                id: None,
                            });
                            let _ = directory.apply(&DirectoryEntry::FriendAccepted {
                                from: ann.clone(),
                                to: other.clone(),
                                request: None,
                                id: None,
                            });
                        }
                        2 => {
                            let _ = directory.remove_friend(&ann_credential, other);
                        }
                        3 => {
                            let _ = directory.remove_friend_as(&ann, other);
                        }
                        _ => {
                            let _ = directory.apply(&DirectoryEntry::FriendRequested {
                                from: ann.clone(),
                                to: other.clone(),
                                id: generated,
                            });
                        }
                    }
                }
                let snapshot = (
                    directory.friends.clone(),
                    directory.friend_requests.clone(),
                    directory.requests.clone(),
                    directory.removed_requests.clone(),
                );
                drop(directory);

                let replayed = Directory::replay_read_only(&path).unwrap();
                prop_assert_eq!(
                    &snapshot,
                    &(
                        replayed.friends.clone(),
                        replayed.friend_requests.clone(),
                        replayed.requests.clone(),
                        replayed.removed_requests.clone(),
                    ),
                    "read-only replay diverged from live state"
                );
                drop(replayed);

                let reopened = Directory::open(&path).unwrap();
                prop_assert_eq!(
                    &snapshot,
                    &(
                        reopened.friends.clone(),
                        reopened.friend_requests.clone(),
                        reopened.requests.clone(),
                        reopened.removed_requests.clone(),
                    ),
                    "writer reopen diverged from live state"
                );
            }
        }
    }

    #[test]
    fn a_newer_legacy_friendship_survives_an_older_canonical_removal_in_replay() {
        // #5624 (Codex P2 on the fix PR): the pair first removes a
        // canonical friendship - the artifact log keeps that removal,
        // naming the canonical request - then befriends again through a
        // legacy noncanonical request. Recovery scans all stored
        // artifacts; the older removal targeted the earlier formation
        // and must not end the current legacy friendship.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        let canonical = "0123456789abcdef0123456789abcdef".to_owned();
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some(canonical.clone()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some(canonical),
                id: None,
            })
            .unwrap();
        assert!(directory.are_friends(&ann, &bo));
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r1".to_owned()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some("r1".to_owned()),
                id: None,
            })
            .unwrap();
        assert!(directory.are_friends(&ann, &bo));
        drop(directory);

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(reopened.are_friends(&ann, &bo));
    }

    #[test]
    fn a_newer_canonical_friendship_survives_an_older_legacy_removal_in_replay() {
        // #5624 (Codex P2 + CodeRabbit on the fix PR): the pair forms a
        // friendship through a legacy noncanonical request and removes
        // it - the artifact names no requests - then befriends again
        // through a canonical request. Recovery scans every stored
        // artifact, including the already-journaled legacy removal; the
        // empty-request legacy signature must not end the newer
        // canonical friendship or void its request ID.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r1".to_owned()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some("r1".to_owned()),
                id: None,
            })
            .unwrap();
        assert!(directory.are_friends(&ann, &bo));
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        let canonical = "0123456789abcdef0123456789abcdef".to_owned();
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some(canonical.clone()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some(canonical.clone()),
                id: None,
            })
            .unwrap();
        assert!(directory.are_friends(&ann, &bo));
        drop(directory);

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(replayed.are_friends(&ann, &bo));
        assert!(!replayed.removed_requests.contains(&canonical));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(reopened.are_friends(&ann, &bo));
        assert!(!reopened.removed_requests.contains(&canonical));
    }

    #[test]
    fn a_newer_pending_request_survives_an_older_canonical_removal_in_replay() {
        // #5624 (Codex P2 on the fix PR): the pair's canonical pending
        // request is removed - the artifact names it - and the pair
        // later creates a new pending request. Applying a removal
        // clears every pending request for the pair, so re-applying
        // the stored removal during recovery would silently cancel
        // the newer request the artifact does not name.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        let first = "0123456789abcdef0123456789abcdef".to_owned();
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some(first.clone()),
            })
            .unwrap();
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(directory.pending_id(&ann, &bo).is_none());
        assert!(directory.removed_requests.contains(&first));
        let second = "fedcba9876543210fedcba9876543210".to_owned();
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some(second.clone()),
            })
            .unwrap();
        assert_eq!(
            directory.pending_id(&ann, &bo).as_deref(),
            Some(second.as_str())
        );
        drop(directory);

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert_eq!(
            replayed.pending_id(&ann, &bo).as_deref(),
            Some(second.as_str())
        );
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert_eq!(
            reopened.pending_id(&ann, &bo).as_deref(),
            Some(second.as_str())
        );
    }

    #[test]
    fn a_newer_id_less_friendship_survives_an_older_legacy_removal_in_replay() {
        // #5624 (Codex P2 on the fix PR): after a legacy-formed
        // friendship is removed (empty-request artifact), the pair
        // befriends again through the local ID-less flow
        // (request_friend_as / accept_friend_as, used by the local UI).
        // The newer friendship has no formed-by ID, so the canonical
        // guard cannot protect it; recovery must skip ID-less
        // formations rather than re-apply the historical removal.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r1".to_owned()),
            })
            .unwrap();
        directory
            .apply(&DirectoryEntry::FriendAccepted {
                from: ann.clone(),
                to: bo.clone(),
                request: Some("r1".to_owned()),
                id: None,
            })
            .unwrap();
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(!directory.are_friends(&ann, &bo));
        directory.request_friend_as(&ann, &bo).unwrap();
        directory.accept_friend_as(&bo, &ann).unwrap();
        assert!(directory.are_friends(&ann, &bo));
        drop(directory);

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(reopened.are_friends(&ann, &bo));
    }

    #[test]
    fn a_newer_id_less_pending_request_survives_an_older_canonical_removal_in_replay() {
        // #5624 (Codex P2 on the fix PR): a canonical pending request
        // is removed (the artifact names it), then the pair creates a
        // new pending request through the local ID-less flow.
        // pending_id flattens the ID-less entry away, so the recovery
        // guard must count the entry itself as pending state the
        // historical removal does not name.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        let canonical = "0123456789abcdef0123456789abcdef".to_owned();
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some(canonical.clone()),
            })
            .unwrap();
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(directory.pending_id(&ann, &bo).is_none());
        directory.request_friend_as(&ann, &bo).unwrap();
        assert!(directory.requests.contains_key(&(ann.clone(), bo.clone())));
        drop(directory);

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(replayed.requests.contains_key(&(ann.clone(), bo.clone())));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(reopened.requests.contains_key(&(ann, bo)));
    }

    #[test]
    fn a_pending_legacy_request_is_cancelled_by_an_empty_removal_artifact_on_receipt() {
        // #5624 (Codex P2 on the fix PR): the producer cancels a pending
        // legacy request locally, but the signed artifact names no
        // requests (the #3146 contract refuses the noncanonical ID), so
        // a receiving server holding the same pending request must
        // cancel its local copy from the pair's legacy state.
        let mut producer = Directory::new();
        let (ann, ann_credential) = account(&mut producer, "Ann");
        let (bo, _bo_credential) = account(&mut producer, "Bo");
        producer
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r2".to_owned()),
            })
            .unwrap();
        let removal = producer.remove_friend(&ann_credential, &bo).unwrap();
        assert!(producer.pending_id(&ann, &bo).is_none());

        let mut receiver = Directory::new();
        receiver
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r2".to_owned()),
            })
            .unwrap();
        assert_eq!(receiver.pending_id(&ann, &bo).as_deref(), Some("r2"));
        receiver.receive_artifact(&removal).unwrap();
        assert!(receiver.pending_id(&ann, &bo).is_none());
        assert!(receiver.removed_requests.contains("r2"));
    }

    #[test]
    fn a_pending_legacy_cancellation_survives_a_lost_journal_entry_in_recovery() {
        // #5624 (Codex P2 on the fix PR): the producer stores the
        // removal artifact, then crashes before journaling the removal
        // entry. Recovery (writer open and read-only replay alike) must
        // still cancel the pending legacy request instead of reviving
        // it as acceptable.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _bo_credential) = account(&mut directory, "Bo");
        directory
            .apply(&DirectoryEntry::FriendRequested {
                from: ann.clone(),
                to: bo.clone(),
                id: Some("r2".to_owned()),
            })
            .unwrap();
        directory.remove_friend(&ann_credential, &bo).unwrap();
        assert!(directory.pending_id(&ann, &bo).is_none());
        drop(directory);

        let text = std::fs::read_to_string(&path).unwrap();
        let kept: String = text
            .lines()
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, kept).unwrap();

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(replayed.pending_id(&ann, &bo).is_none());
        assert!(replayed.removed_requests.contains("r2"));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(reopened.pending_id(&ann, &bo).is_none());
        assert!(reopened.removed_requests.contains("r2"));
    }

    #[test]
    fn a_friend_removal_cannot_void_another_pairs_pending_request() {
        let mut receiver = Directory::new();
        let (ann, ann_credential) = account(&mut receiver, "Ann");
        let (bo, _) = account(&mut receiver, "Bo");
        let (cy, cy_credential) = account(&mut receiver, "Cy");
        let (di, di_credential) = account(&mut receiver, "Di");
        // The receiver already holds Cy/Di's pending request, so its ID is
        // known to belong to another pair.
        let (genuine, _) = receiver
            .request_friend(&cy_credential, &di, "receiver", 1)
            .unwrap();

        // Ann signs a well-formed removal for Ann/Bo but names Cy/Di's
        // pending envelope. The receiver stores the artifact but must not
        // void the foreign ID.
        let removal = SignedArtifact::sign(
            &key_of(&ann_credential),
            1,
            "",
            artifact::FRIEND_REMOVE,
            json!({
                "a": ann,
                "b": bo,
                "requests": [genuine.id.clone()]
            }),
        )
        .unwrap();

        assert_eq!(receiver.receive_artifact(&removal).unwrap(), Stored::New);
        assert!(!receiver.removed_requests.contains(&genuine.id));
        assert!(!receiver.are_friends(&ann, &bo));
        // The genuine pending request survives: Di can still accept it.
        receiver
            .accept_friend(&di_credential, &cy, "receiver", 2)
            .unwrap();
        assert!(receiver.are_friends(&cy, &di));
    }

    #[test]
    fn receive_artifact_refuses_friend_removal_with_too_many_requests_without_storing_it() {
        let mut directory = Directory::new();
        let (ann, ann_credential) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let valid_id = |n: u8| format!("{n:032x}");
        let cases = [
            vec![valid_id(1), valid_id(2), valid_id(3), valid_id(4)],
            vec!["g".repeat(32)],
            vec!["a".repeat(31)],
            vec!["a".repeat(33)],
        ];

        for requests in cases {
            let removal = SignedArtifact::sign_unchecked(
                &key_of(&ann_credential),
                1,
                "",
                artifact::FRIEND_REMOVE,
                json!({
                    "a": ann.clone(),
                    "b": bo.clone(),
                    "requests": requests
                }),
            )
            .unwrap();
            let before = directory.clone();

            let error = directory.receive_artifact(&removal).unwrap_err();
            assert!(
                matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Forbidden(_))),
                "{error:#}"
            );
            assert_eq!(directory, before);
        }
    }

    #[test]
    fn concurrent_removals_of_one_friendship_by_both_sides_both_apply_and_end_it_once() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        home.receive(&accept, "home", 2).unwrap();

        let by_ann = home.remove_friend(&ann_credential, &bo).unwrap();
        let by_bo = away.remove_friend(&bo_credential, &ann).unwrap();

        assert_eq!(home.receive_artifact(&by_bo).unwrap(), Stored::New);
        assert_eq!(away.receive_artifact(&by_ann).unwrap(), Stored::New);
        assert!(!home.are_friends(&ann, &bo));
        assert!(!away.are_friends(&ann, &bo));
        assert_eq!(home.receive_artifact(&by_bo).unwrap(), Stored::Duplicate);
        assert_eq!(away.receive_artifact(&by_ann).unwrap(), Stored::Duplicate);
    }

    #[test]
    fn an_unknown_kind_is_stored_and_changes_nothing() {
        let mut home = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let mut away = Directory::new();
        let future = SignedArtifact::sign(
            &key_of(&ann_credential),
            1,
            "",
            "future_kind",
            json!({"x": 1}),
        )
        .unwrap();
        assert_eq!(away.receive_artifact(&future).unwrap(), Stored::New);
        assert_eq!(away.artifacts().head_seq(&ann), 1);
        assert!(away.friends_of(&ann).is_empty());
        assert_eq!(away.receive_artifact(&future).unwrap(), Stored::Duplicate);
    }

    #[test]
    fn a_newer_home_declaration_replaces_an_older_one() {
        let mut home = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let first = home
            .declare_home(&ann_credential, vec![home_server(1)])
            .unwrap();
        home.declare_home(&ann_credential, vec![home_server(2)])
            .unwrap();
        let declared = |home: &Directory| {
            home.artifacts().declared(&ann).unwrap().servers[0]
                .endpoint
                .clone()
        };
        assert_eq!(declared(&home), home_server(2).endpoint);
        assert_eq!(home.receive_artifact(&first).unwrap(), Stored::Duplicate);
        assert_eq!(declared(&home), home_server(2).endpoint);
    }

    #[test]
    fn declare_home_refuses_what_receipt_would_refuse() {
        let mut home = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        // The same endpoint named twice.
        assert!(
            home.declare_home(&ann_credential, vec![home_server(1), home_server(1)])
                .is_err()
        );
        // A world ID past the receipt bound.
        let mut oversized = home_server(1);
        oversized.world_id = "w".repeat(65);
        assert!(home.declare_home(&ann_credential, vec![oversized]).is_err());
        assert_eq!(home.artifacts().head_seq(&ann), 0);
        // A well-formed declaration still publishes.
        home.declare_home(&ann_credential, vec![home_server(1)])
            .unwrap();
        assert_eq!(home.artifacts().head_seq(&ann), 1);
    }

    #[test]
    fn a_removal_survives_a_restart_and_is_reapplied_when_its_directory_line_was_lost() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let (mut home, _away, ann, bo, ann_credential) = friends_across_servers(Some(&path));
        home.remove_friend(&ann_credential, &bo).unwrap();
        drop(home);

        let reopened = Directory::open(&path).unwrap();
        assert!(!reopened.are_friends(&ann, &bo));
        assert_eq!(reopened.artifacts().head_seq(&ann), 1);
        drop(reopened);

        let text = std::fs::read_to_string(&path).unwrap();
        let kept: String = text
            .lines()
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, kept).unwrap();
        let crashed = Directory::open(&path).unwrap();
        assert!(!crashed.are_friends(&ann, &bo));
        assert_eq!(crashed.artifacts().head_seq(&ann), 1);
    }

    #[test]
    fn a_legacy_removal_with_duplicate_request_ids_still_ends_the_friendship_on_recovery() {
        // Codex P2 on PR #5576: a removal stored before receipt refused
        // duplicate request IDs (#3146) must still take effect when its
        // journal entry was lost to a crash. Strict `parsed` rejects the
        // legacy artifact, so both recovery scans (writer open and
        // read-only replay) must interpret it with `parsed_for_replay`,
        // like the artifact log's own replay path.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let (home, _away, ann, bo, ann_credential) = friends_across_servers(Some(&path));
        let requests = home.friendship_requests(&ann, &bo);
        assert_eq!(requests.len(), 1);
        let request = requests[0].clone();
        drop(home);

        let legacy = SignedArtifact::sign_legacy_unchecked(
            &key_of(&ann_credential),
            1,
            "",
            artifact::FRIEND_REMOVE,
            json!({
                "a": ann,
                "b": bo,
                "requests": [request.clone(), request.clone()]
            }),
        )
        .unwrap();
        assert!(legacy.parsed().is_err());
        artifact::append(&artifact_path(&path), &legacy).unwrap();

        let replayed = Directory::replay_read_only(&path).unwrap();
        assert!(!replayed.are_friends(&ann, &bo));
        drop(replayed);

        let reopened = Directory::open(&path).unwrap();
        assert!(!reopened.are_friends(&ann, &bo));
        assert!(reopened.removed_requests.contains(&request));
    }

    #[test]
    fn artifacts_hold_no_secret_digest_or_email() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let (mut home, _away, ann, bo, ann_credential) = friends_across_servers(Some(&path));
        home.confirm_email(Actor::Server, &ann, "ann@example.com")
            .unwrap();
        home.remove_friend(&ann_credential, &bo).unwrap();
        home.declare_home(&ann_credential, vec![home_server(1)])
            .unwrap();

        let secret = ann_credential.rsplit('.').next().unwrap();
        let digest = hex_encode(&Sha256::digest(hex_to_array::<32>(secret).unwrap()));
        let stored = std::fs::read_to_string(dir.path().join(artifact::FILE)).unwrap();
        assert!(!stored.contains(secret));
        assert!(!stored.contains(&digest));
        assert!(!stored.contains('@'));
        assert_eq!(stored.lines().count(), 2);
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
            .set_member(Actor::Account(&bo), group, cy_member, Some(Level::Write))
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
        let di_member = Member::Account { id: di };
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
    fn diamond_nesting_preserves_max_of_min_and_checks_member_cycles() {
        let (mut directory, bo, layers, holder) = diamond_directory();
        let root = layers.last().unwrap()[0];

        assert_eq!(directory.level_in(&bo, root), Some(Level::Write));

        directory
            .set_member(
                Actor::Server,
                holder,
                Member::Group { id: root },
                Some(Level::Read),
            )
            .unwrap();
        assert_eq!(directory.level_in(&bo, holder), Some(Level::Read));
        assert!(
            directory
                .set_member(
                    Actor::Server,
                    root,
                    Member::Group { id: holder },
                    Some(Level::Read),
                )
                .is_err(),
            "the MemberSet path still refuses a cycle"
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
        let (request, _) = directory
            .request_friend(&ann_credential, &bo, "home", 1)
            .unwrap();
        assert!(request.verify().is_ok());
        assert!(
            directory
                .request_friend(&bo_credential, &ann, "home", 2)
                .is_err(),
            "a request is already pending"
        );
        let (accept, _) = directory
            .accept_friend(&bo_credential, &ann, "home", 3)
            .unwrap();
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
        let (mut request, _) = directory
            .request_friend(&ann_credential, &bo, "home", 1)
            .unwrap();
        request.at = 2;
        assert!(request.verify().is_err());
    }

    #[test]
    fn friendship_crosses_servers_with_envelopes_alone() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        home.receive(&accept, "home", 2).unwrap();
        assert_eq!(home.friends_of(&ann), vec![bo.clone()]);
        assert_eq!(away.friends_of(&bo), vec![ann.clone()]);
        assert!(
            away.receive(&accept, "away", 2).is_err(),
            "an acceptance for another server is refused"
        );
        let mut stranger = Directory::new();
        assert!(
            stranger.receive(&request, "away", 1).is_err(),
            "a request for an account this server does not hold is refused"
        );
    }

    fn key_of(credential: &str) -> SigningKey {
        let secret = hex_to_array::<32>(credential.rsplit('.').next().unwrap()).unwrap();
        signing_key(&secret)
    }

    #[test]
    fn a_stale_acceptance_cannot_befriend_after_a_new_request() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        let second = Envelope::signed(
            &key_of(&bo_credential),
            EnvelopeKind::FriendAccept,
            &bo,
            &ann,
            "home",
            Some(&request.id),
            2,
        )
        .unwrap();
        home.receive(&accept, "home", 2).unwrap();
        away.remove_friend(&bo_credential, &ann).unwrap();
        home.remove_friend(&ann_credential, &bo).unwrap();
        home.request_friend(&ann_credential, &bo, "away", 3)
            .unwrap();
        let stale = home.receive(&second, "home", 3).unwrap_err();
        assert!(
            matches!(stale.downcast_ref::<Refusal>(), Some(Refusal::Conflict(_))),
            "an acceptance of a consumed request is refused: {stale:#}"
        );
        assert_eq!(
            home.receive(&accept, "home", 3).unwrap(),
            Received::Duplicate,
            "an applied acceptance replayed is a no-op"
        );
        assert!(!home.are_friends(&ann, &bo));
    }

    #[test]
    fn a_repeated_envelope_is_a_duplicate_and_changes_nothing() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, _) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        assert!(matches!(
            away.receive(&request, "away", 1).unwrap(),
            Received::Applied(_)
        ));
        assert_eq!(
            away.receive(&request, "away", 1).unwrap(),
            Received::Duplicate
        );
        assert_eq!(away.pending_for(&bo).0, vec![ann]);
    }

    #[test]
    fn an_envelope_for_another_server_or_outside_its_window_is_refused() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (_, ann_credential) = account(&mut home, "Ann");
        let (bo, _) = account(&mut away, "Bo");
        let at = ENVELOPE_TTL_MS;
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", at)
            .unwrap();
        assert!(away.receive(&request, "elsewhere", at).is_err());
        assert!(
            away.receive(&request, "away", at + ENVELOPE_TTL_MS + 1)
                .is_err()
        );
        assert!(
            away.receive(&request, "away", at - ENVELOPE_SKEW_MS - 1)
                .is_err()
        );
        assert!(away.receive(&request, "away", at).is_ok());
    }

    #[test]
    fn an_acceptance_must_name_its_request() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, _) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let unnamed = Envelope::signed(
            &key_of(&bo_credential),
            EnvelopeKind::FriendAccept,
            &bo,
            &ann,
            "home",
            None,
            2,
        )
        .unwrap();
        assert!(home.receive(&unnamed, "home", 2).is_err());
    }

    #[test]
    fn a_sender_has_at_most_twenty_requests_pending_on_a_server() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (_, ann_credential) = account(&mut home, "Ann");
        for n in 0..=MAX_PENDING_FRIEND_REQUESTS {
            let (bo, _) = account(&mut away, &format!("Bo {n}"));
            let (request, _) = home
                .request_friend(&ann_credential, &bo, "away", 1)
                .unwrap();
            let received = away.receive(&request, "away", 1);
            if n < MAX_PENDING_FRIEND_REQUESTS {
                assert!(matches!(received.unwrap(), Received::Applied(_)));
            } else {
                let refused = received.unwrap_err();
                assert!(
                    matches!(
                        refused.downcast_ref::<Refusal>(),
                        Some(Refusal::Forbidden(_))
                    ),
                    "the request past the cap is refused: {refused:#}"
                );
            }
        }
    }

    #[test]
    fn applied_envelopes_are_still_seen_after_a_restart() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut away = Directory::open(&path).unwrap();
        let (bo, _) = account(&mut away, "Bo");
        let mut home = Directory::new();
        let (_, ann_credential) = account(&mut home, "Ann");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let mut reopened = Directory::open(&path).unwrap();
        assert_eq!(
            reopened.receive(&request, "away", 1).unwrap(),
            Received::Duplicate
        );
    }

    #[test]
    fn friend_lines_from_before_envelopes_still_replay() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::open(&path).unwrap();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let requested = format!(r#"{{"entry":"friend_requested","from":"{ann}","to":"{bo}"}}"#);
        let accepted = format!(r#"{{"entry":"friend_accepted","from":"{ann}","to":"{bo}"}}"#);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text = format!("{text}{requested}\n{accepted}\n");
        std::fs::write(&path, text).unwrap();
        assert!(Directory::replay(&path).unwrap().are_friends(&ann, &bo));
    }

    #[test]
    fn levels_keep_their_order_and_wire_names() {
        let order = [
            (Level::Access, "access"),
            (Level::Read, "read"),
            (Level::Write, "write"),
            (Level::Manage, "manage"),
            (Level::Moderator, "moderator"),
            (Level::Admin, "admin"),
        ];
        for pair in order.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{:?} < {:?}", pair[0].0, pair[1].0);
        }
        for (level, name) in order {
            assert_eq!(
                serde_json::to_string(&level).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(
                serde_json::from_str::<Level>(&format!("\"{name}\"")).unwrap(),
                level
            );
        }
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

    #[test]
    fn replay_drops_a_torn_final_fragment_and_truncates_the_journal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::new();
        let (ann, ann_credential) = account_into(&mut directory, &path, "José");
        let (_bo, _) = account_into(&mut directory, &path, "Bo");
        let complete = std::fs::read(&path).unwrap();

        // A crash after the first byte of a multibyte name leaves invalid
        // UTF-8 in the unterminated final entry.
        let mut torn = complete.clone();
        torn.extend_from_slice(b"{\"name\":\"Jos");
        torn.push(0xc3); // First byte of "é".
        std::fs::write(&path, torn).unwrap();

        let mut replayed = Directory::replay(&path).unwrap();
        assert_eq!(replayed.account_for(&ann_credential), Some(ann));
        // The fragment is gone: the file is back to the complete lines.
        assert_eq!(std::fs::read(&path).unwrap(), complete);
        let (carol, carol_credential) = account_into(&mut replayed, &path, "Carol");
        let roundtrip = Directory::replay(&path).unwrap();
        assert_eq!(roundtrip.account_for(&carol_credential), Some(carol));
    }

    #[test]
    fn replay_drops_a_parseable_but_unterminated_final_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::new();
        let (_ann, _) = account_into(&mut directory, &path, "Ann");
        let complete = std::fs::read_to_string(&path).unwrap();

        // A fully-formed entry whose newline never landed was never
        // acknowledged, so it is dropped like any other torn append.
        let mut staging = Directory::new();
        let (cy_credential, entry) = staging.sign_up("Cy").unwrap();
        let mut text = complete.clone();
        text.push_str(&serde_json::to_string(&entry).unwrap());
        std::fs::write(&path, text).unwrap();

        let replayed = Directory::replay(&path).unwrap();
        assert_eq!(replayed.account_for(&cy_credential), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), complete);
    }

    #[test]
    fn replay_refuses_a_terminated_corrupt_final_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("directory.jsonl");
        let mut directory = Directory::new();
        let (_ann, _) = account_into(&mut directory, &path, "Ann");

        // Newline-terminated but unparseable is corruption, never a
        // torn append: it must still fail as malformed.
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("not json\n");
        std::fs::write(&path, text).unwrap();
        let error = Directory::replay(&path)
            .err()
            .expect("corrupt line must fail");
        assert!(
            format!("{error:#}").contains("is malformed"),
            "unexpected error: {error:#}"
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
        let (mut request, _) = directory
            .request_friend(&cy_credential, &bo, "home", 1)
            .unwrap();
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

    #[test]
    fn plus_encoded_hex_is_refused() {
        // #2517: '+' pairs decode identically to '0' pairs under
        // from_str_radix, so they must be refused at the gate — the same
        // bytes must have exactly one accepted spelling.
        assert!(hex_to_array::<32>(&"+a".repeat(32)).is_none());
        assert!(hex_to_array::<32>(&"+7".repeat(32)).is_none());
        // A single '+' pair anywhere poisons the whole input.
        let mut mixed = "0a".repeat(32);
        mixed.replace_range(10..12, "+b");
        assert!(hex_to_array::<32>(&mixed).is_none());
        // Canonical hex in either case still decodes, identically.
        assert_eq!(
            hex_to_array::<32>(&"0a0b0c0d".repeat(8)),
            hex_to_array::<32>(&"0A0B0C0D".repeat(8))
        );
        assert!(hex_to_array::<32>(&"0a0b0c0d".repeat(8)).is_some());
    }

    fn public_key(seed: u8) -> String {
        let key = SigningKey::from_bytes(&[seed; 32]);
        hex_encode(&key.verifying_key().to_bytes())
    }

    fn refusal(error: &anyhow::Error) -> &Refusal {
        error.downcast_ref::<Refusal>().unwrap()
    }

    #[test]
    fn a_keyed_account_signs_up_without_an_email() {
        let mut directory = Directory::new();
        let key = public_key(1);
        directory.sign_up_keyed(&key, "Kay").unwrap();
        assert_eq!(directory.account_for_key(&key), Some(key.clone()));
        assert!(!directory.is_verified(&key));
        assert_eq!(directory.ceiling(&key), WorldScopes::GUEST);
    }

    #[test]
    fn a_key_belongs_to_one_account_and_is_added_and_removed_by_its_owner() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let laptop = public_key(2);
        directory
            .add_key(Actor::Account(&ann), &ann, &laptop)
            .unwrap();
        assert_eq!(directory.account_for_key(&laptop), Some(ann.clone()));

        let error = directory
            .add_key(Actor::Account(&bo), &ann, &public_key(3))
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Forbidden(_)));
        let error = directory
            .add_key(Actor::Account(&bo), &bo, &laptop)
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Conflict(_)));

        let error = directory
            .remove_key(Actor::Account(&bo), &ann, &laptop)
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Forbidden(_)));
        directory
            .remove_key(Actor::Account(&ann), &ann, &laptop)
            .unwrap();
        assert_eq!(directory.account_for_key(&laptop), None);
        assert_eq!(directory.account_for_key(&ann), Some(ann.clone()));
    }

    #[test]
    fn a_key_already_signed_up_with_cannot_sign_up_again() {
        let mut directory = Directory::new();
        let key = public_key(4);
        directory.sign_up_keyed(&key, "Kay").unwrap();
        let error = directory.sign_up_keyed(&key, "Other").unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Conflict(_)));
        let error = directory.sign_up_keyed("zz", "Bad").unwrap_err();
        assert!(error.downcast_ref::<Refusal>().is_none());
    }

    #[test]
    fn only_the_server_confirms_an_email_and_confirming_verifies_the_account() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        let error = directory
            .confirm_email(Actor::Account(&ann), &ann, "ann@example.com")
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Forbidden(_)));

        directory
            .confirm_email(Actor::Server, &ann, "  Ann@Example.COM ")
            .unwrap();
        assert_eq!(
            directory.account_for_email("ann@example.com"),
            Some(ann.clone())
        );
        assert!(directory.is_verified(&ann));
        assert!(directory.ceiling(&ann).contains(WorldScopes::DELEGATE));

        let error = directory
            .confirm_email(Actor::Server, &bo, "ANN@example.com")
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Conflict(_)));
        assert!(!directory.is_verified(&bo));
    }

    #[test]
    fn reconfirming_the_same_accounts_email_is_an_idempotent_no_op() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        directory
            .confirm_email(Actor::Server, &ann, "ann@example.com")
            .unwrap();
        let emails_before = directory.account(&ann).unwrap().emails.clone();
        directory
            .confirm_email(Actor::Server, &ann, "ANN@example.com")
            .unwrap();
        assert_eq!(directory.account(&ann).unwrap().emails, emails_before);
        assert_eq!(
            directory.account_for_email("ann@example.com"),
            Some(ann.clone())
        );

        let error = directory
            .confirm_email(Actor::Server, &bo, "ann@example.com")
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Conflict(_)));
    }

    #[test]
    fn an_email_is_removed_only_by_its_owner_and_a_missing_one_is_not_found() {
        let mut directory = Directory::new();
        let (ann, _) = account(&mut directory, "Ann");
        let (bo, _) = account(&mut directory, "Bo");
        directory
            .confirm_email(Actor::Server, &ann, "ann@example.com")
            .unwrap();
        let error = directory
            .remove_email(Actor::Account(&bo), &ann, "ann@example.com")
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::Forbidden(_)));
        directory
            .remove_email(Actor::Account(&ann), &ann, "ann@example.com")
            .unwrap();
        assert_eq!(directory.account_for_email("ann@example.com"), None);
        assert!(!directory.is_verified(&ann));
        let error = directory
            .remove_email(Actor::Account(&ann), &ann, "ann@example.com")
            .unwrap_err();
        assert!(matches!(refusal(&error), Refusal::NotFound(_)));
    }

    #[test]
    fn a_malformed_email_is_refused() {
        assert!(normalize_email("no-at-sign").is_err());
        assert!(normalize_email("two@@example.com").is_err());
        assert!(normalize_email("a@nodot").is_err());
        assert!(normalize_email("ok@example.com").is_ok());
    }

    #[test]
    fn an_envelope_with_a_non_canonical_s_is_refused() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let mut envelope = Envelope::signed(
            &key,
            EnvelopeKind::FriendRequest,
            &public_key(7),
            &public_key(8),
            "home",
            None,
            1,
        )
        .unwrap();
        assert!(envelope.verify().is_ok());
        let raw = hex_to_array::<64>(&envelope.signature).unwrap();
        envelope.signature = hex_encode(&with_non_canonical_s(raw));
        assert!(envelope.verify().is_err());
    }

    #[test]
    fn an_envelope_under_a_small_order_key_is_refused() {
        let envelope = Envelope {
            kind: EnvelopeKind::FriendRequest,
            from: hex_encode(&IDENTITY_KEY),
            to: public_key(8),
            audience: "home".to_owned(),
            id: "e1".to_owned(),
            request: None,
            at: 1,
            signature: hex_encode(&identity_forgery()),
        };
        assert!(envelope.verify().is_err());
    }
}
