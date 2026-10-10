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
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// No valid credential, session or key was presented.
    #[error("{0}")]
    Unauthenticated(String),
    /// The caller is known but may not do this.
    #[error("{0}")]
    Forbidden(String),
    /// The named account, group or member does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The change clashes with what already exists.
    #[error("{0}")]
    Conflict(String),
    /// Too many requests for the same thing in a short time.
    #[error("{0}")]
    RateLimited(String),
}

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

/// Longest time after it is made that an envelope is accepted.
const ENVELOPE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// How far ahead of the receiving clock an envelope may be dated.
const ENVELOPE_SKEW_MS: u64 = 5 * 60 * 1000;

/// Friend requests one sender account may have pending on a server at once.
pub const MAX_PENDING_FRIEND_REQUESTS: usize = 20;

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
    /// The world ID of the server the envelope is for.
    pub audience: String,
    /// A random ID unique to this envelope. A request's ID names the request.
    pub id: String,
    /// For an acceptance, the ID of the request it accepts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
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
    audience: &'a str,
    id: &'a str,
    request: Option<&'a str>,
    at: u64,
}

impl Envelope {
    fn body(&self) -> SignedBody<'_> {
        SignedBody {
            kind: self.kind,
            from: &self.from,
            to: &self.to,
            audience: &self.audience,
            id: &self.id,
            request: self.request.as_deref(),
            at: self.at,
        }
    }

    fn signed(
        key: &SigningKey,
        kind: EnvelopeKind,
        from: &str,
        to: &str,
        audience: &str,
        request: Option<&str>,
        at: u64,
    ) -> Result<Self> {
        let mut nonce = [0_u8; 16];
        rand::rng().fill(&mut nonce);
        let mut envelope = Self {
            kind,
            from: from.to_owned(),
            to: to.to_owned(),
            audience: audience.to_owned(),
            id: hex_encode(&nonce),
            request: request.map(str::to_owned),
            at,
            signature: String::new(),
        };
        let signature = key.sign(&serde_json::to_vec(&envelope.body())?);
        envelope.signature = hex_encode(&signature.to_bytes());
        Ok(envelope)
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
        let body = serde_json::to_vec(&self.body())?;
        key.verify(&body, &signature)
            .context("envelope signature does not verify")
    }
}

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
    requests: BTreeMap<(String, String), Option<String>>,
    seen: BTreeSet<String>,
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
            requests: BTreeMap::new(),
            seen: BTreeSet::new(),
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
        self.apply(&entry)?;
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
        self.apply(&entry)?;
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

    fn insert_account(&mut self, id: &str, name: &str) -> Result<()> {
        self.accounts.insert(
            id.to_owned(),
            Account {
                id: id.to_owned(),
                name: clean_name(name)?,
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

    pub(crate) fn apply(&mut self, entry: &DirectoryEntry) -> Result<()> {
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
                refuse_unless!(
                    !self.accounts.contains_key(id),
                    Conflict,
                    "account already exists"
                );
                let digest = hex_to_array::<32>(digest).context("malformed account digest")?;
                self.insert_account(id, name)?;
                self.digests.insert(id.clone(), digest);
            }
            DirectoryEntry::AccountKeyed { id, name } => {
                ensure_account_id(id)?;
                refuse_unless!(
                    self.account_for_key(id).is_none(),
                    Conflict,
                    "that key already belongs to an account"
                );
                self.insert_account(id, name)?;
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
                refuse_unless!(
                    self.account_for_email(email).is_none(),
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
                        name: clean_name(name)?,
                        description: clean_description(description)?,
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
                let name = clean_name(name)?;
                let description = clean_description(description)?;
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

pub(crate) fn hex_to_array<const N: usize>(text: &str) -> Option<[u8; N]> {
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
}
