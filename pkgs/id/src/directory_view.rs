//! What a viewer may see of a server's directory, shaped once for every transport.
//!
//! HTTP serializes a [`DirectoryView`] as JSON and SSH renders it with
//! [`render_text`]. Visibility is decided here alone, so an entity shown on one
//! transport is shown on the other.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::directory::{
    Actor, Directory, DirectoryEntry, Envelope, EnvelopeKind, Level, Member, Received, Refusal,
    normalize_email, normalize_key,
};
use crate::directory_auth::{Caller, DirectoryAuth, Principal, Purpose};
use crate::directory_mail::Mail;
use crate::envelope_outbox::{Entry, Finish, Outbox, check_url};
use crate::world::WorldScopes;

/// Who is looking at the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    /// No proof: account names and public groups only.
    Anonymous,
    /// An account, by its ID.
    Account(String),
    /// The admin token: everything.
    Admin,
}

impl Viewer {
    /// The viewer a caller is. The admin token wins, then an account
    /// credential, then a session, then a signing key. A key no account holds
    /// reads as anonymous, since it may be about to sign up.
    ///
    /// # Errors
    ///
    /// Fails when a credential or session presented names no current account.
    pub fn resolve(
        directory: &Directory,
        auth: &mut DirectoryAuth,
        caller: &Caller,
        now: u64,
    ) -> Result<Self> {
        if caller.admin {
            return Ok(Self::Admin);
        }
        if let Some(credential) = &caller.credential {
            return directory
                .account_for(credential)
                .map(Self::Account)
                .ok_or_else(|| anyhow!(Refusal::Unauthenticated("unknown credential".to_owned())));
        }
        if let Some(token) = &caller.session {
            return match auth.principal(token, now) {
                Some(Principal::Admin) => Ok(Self::Admin),
                Some(Principal::Account(id)) if directory.account(&id).is_some() => {
                    Ok(Self::Account(id))
                }
                _ => Err(anyhow!(Refusal::Unauthenticated(
                    "session has ended; sign in again".to_owned()
                ))),
            };
        }
        if let Some(key) = &caller.key {
            return Ok(directory
                .account_for_key(key)
                .map_or(Self::Anonymous, Self::Account));
        }
        Ok(Self::Anonymous)
    }
}

/// What one viewer may see of the directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryView {
    /// `admin`, `anonymous`, or the viewer's account ID.
    pub viewer: String,
    /// Every account's ID and name. Details appear only for the account itself and the admin.
    pub accounts: Vec<AccountView>,
    /// Public groups, every group the viewer belongs to, and every group for the admin.
    pub groups: Vec<GroupView>,
    /// The viewer's own envelopes still in, or recently left, the outbox.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<DeliveryView>,
}

/// An account as one viewer sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountView {
    /// The account ID.
    pub id: String,
    /// The display name.
    pub name: String,
    /// Whether the account is verified, by an admin or a confirmed email; shown to the account and the admin.
    pub verified: Option<bool>,
    /// Public keys the account signs in with, besides its own ID; shown to the account and the admin.
    pub keys: Option<Vec<String>>,
    /// Confirmed email addresses; shown to the account and the admin.
    pub emails: Option<Vec<String>>,
    /// The permissions the account holds; shown to the account and the admin.
    pub scopes: Option<Vec<String>>,
    /// The account's friends; shown to the account and the admin.
    pub friends: Option<Vec<String>>,
    /// Friend requests received; shown to the account and the admin.
    pub incoming: Option<Vec<String>>,
    /// Friend requests sent; shown to the account and the admin.
    pub outgoing: Option<Vec<String>>,
}

/// A group as one viewer sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupView {
    /// The group ID.
    pub id: u64,
    /// The group name.
    pub name: String,
    /// The group description.
    pub description: String,
    /// Whether anonymous viewers may see the group.
    pub public: bool,
    /// The permissions members gain from the group.
    pub scopes: Vec<String>,
    /// The viewer's own level in the group.
    pub level: Option<Level>,
    /// Direct members with their levels; shown at `read` or better, and to the admin.
    pub members: Option<Vec<MemberView>>,
}

/// A direct member of a group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberView {
    /// The member: an account or a nested group.
    pub member: Member,
    /// The member's name.
    pub name: String,
    /// The member's level in the group.
    pub level: Level,
}

/// A request against the directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DirectoryAction {
    /// Read what the viewer may see.
    View,
    /// Create an account with a credential, returned once.
    SignUp {
        /// The display name.
        name: String,
    },
    /// Create an account identified by the key the request was signed with, with no email.
    SignUpKey {
        /// The display name.
        name: String,
    },
    /// Turn the viewer's account or admin proof into a session token.
    OpenSession,
    /// End the session the request presented.
    SignOut,
    /// Add a key to the viewer's account. The request must be signed with that key.
    AddKey {
        /// The public key, as hex.
        key: String,
    },
    /// Remove a key the viewer's account added. Its own ID stays.
    RemoveKey {
        /// The public key, as hex.
        key: String,
    },
    /// Mail a confirmation code for an address to the viewer's account.
    AddEmail {
        /// The address.
        address: String,
    },
    /// Confirm an address with the code mailed for it.
    ConfirmEmail {
        /// The address.
        address: String,
        /// The six-digit code.
        code: String,
    },
    /// Remove an address from the viewer's account.
    RemoveEmail {
        /// The address.
        address: String,
    },
    /// Mail a sign-in code if the address belongs to an account. The answer is the same either way.
    SignInEmail {
        /// The address.
        address: String,
    },
    /// Trade a sign-in code for a session.
    ConfirmSignIn {
        /// The address.
        address: String,
        /// The six-digit code.
        code: String,
    },
    /// Create a group the viewer founds.
    CreateGroup {
        /// The group name.
        name: String,
        /// The group description.
        description: String,
        /// Permission names the group grants.
        scopes: Vec<String>,
    },
    /// Rename a group, redescribe it, or change its permissions.
    UpdateGroup {
        /// The group ID.
        group: u64,
        /// The group name.
        name: String,
        /// The group description.
        description: String,
        /// Permission names the group grants.
        scopes: Vec<String>,
    },
    /// Show a group to anonymous viewers, or hide it again.
    SetPublic {
        /// The group ID.
        group: u64,
        /// Whether anonymous viewers may see the group.
        public: bool,
    },
    /// Delete a group.
    DeleteGroup {
        /// The group ID.
        group: u64,
    },
    /// Add a member to a group at a level, or remove it with no level.
    SetMember {
        /// The group ID.
        group: u64,
        /// The account or nested group.
        member: Member,
        /// The level to hold, or none to remove.
        level: Option<Level>,
    },
    /// Ask an account to be friends. A remote account is signed for with the
    /// viewer's credential and queued for its server.
    RequestFriend {
        /// The account asked.
        to: String,
        /// Where the account lives, if it is on another server.
        remote: Option<RemoteFriend>,
    },
    /// Accept a friend request. A remote requester is answered through its
    /// server, signed with the viewer's credential.
    AcceptFriend {
        /// The account that asked.
        from: String,
        /// Where the requester lives, if it is on another server.
        remote: Option<RemoteFriend>,
    },
    /// Remove a friend, or withdraw a request.
    RemoveFriend {
        /// The other account.
        other: String,
    },
    /// Apply a friend envelope another server signed, for a local account.
    Receive {
        /// The envelope.
        envelope: Envelope,
    },
    /// Verify an account. Admin only.
    Verify {
        /// The account ID.
        account: String,
    },
}

/// Where an account on another server is reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFriend {
    /// The other server's base URL. Envelopes go to its `/envelope` route.
    pub server: String,
    /// The other server's world ID, which the envelope is addressed to.
    pub audience: String,
}

impl DirectoryAction {
    /// Whether the action sends mail, and so needs a mail sink configured.
    #[must_use]
    pub const fn sends_mail(&self) -> bool {
        matches!(self, Self::AddEmail { .. } | Self::SignInEmail { .. })
    }

    /// Whether the action queues an envelope for another server.
    #[must_use]
    pub const fn sends_envelope(&self) -> bool {
        matches!(
            self,
            Self::RequestFriend {
                remote: Some(_),
                ..
            } | Self::AcceptFriend {
                remote: Some(_),
                ..
            }
        )
    }
}

/// An envelope an action signed, for the caller to queue for another server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outbound {
    /// The `/envelope` endpoint on the other server.
    pub url: String,
    /// The signed envelope.
    pub envelope: Envelope,
}

/// What became of one of the viewer's envelopes in the outbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryView {
    /// `friend request` or `friend acceptance`.
    pub kind: String,
    /// The account the envelope is for.
    pub to: String,
    /// `not yet delivered`, `delivered`, `refused: REASON`, or `gave up: REASON`.
    pub status: String,
}

/// The viewer's own envelopes in the outbox. The admin sees all of them.
#[must_use]
pub fn deliveries(outbox: &Outbox, viewer: &str) -> Vec<DeliveryView> {
    outbox
        .entries()
        .filter(|entry| match viewer {
            "admin" => true,
            "anonymous" => false,
            account => entry.envelope.from == account,
        })
        .map(delivery)
        .collect()
}

fn delivery(entry: &Entry) -> DeliveryView {
    let kind = match entry.envelope.kind {
        EnvelopeKind::FriendRequest => "friend request",
        EnvelopeKind::FriendAccept => "friend acceptance",
    };
    let reason = entry.last_error.as_deref().unwrap_or("no reason given");
    let status = match entry.finish {
        None => "not yet delivered".to_owned(),
        Some(Finish::Delivered) => "delivered".to_owned(),
        Some(Finish::Refused) => format!("refused: {reason}"),
        Some(Finish::GaveUp) => format!("gave up: {reason}"),
    };
    DeliveryView {
        kind: kind.to_owned(),
        to: entry.envelope.to.clone(),
        status,
    }
}

/// One line typed at an explorer.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// Nothing but spaces.
    Blank,
    /// `help`.
    Help,
    /// `quit` or `exit`.
    Quit,
    /// A directory action.
    Action(DirectoryAction),
}

/// The commands the SSH explorer accepts, one per line.
pub const HELP: &str = "\
commands:
  view                                show the directory
  signup NAME                         create an account; its credential is printed once
  keysignup NAME                      create an account for the key this login uses
  session                             open a session as the current account
  signout                             end the session, and drop admin rights for this login
  addkey HEX | removekey HEX          add or remove a key on your account
  addemail ADDR | removeemail ADDR    mail a code to add an address, or remove one
  confirmemail ADDR CODE              confirm an address with its mailed code
  signin ADDR                         mail a sign-in code to a known address
  confirmsignin ADDR CODE             trade the code for a session
  group new SCOPES NAME | DESC        create a group; SCOPES is comma-separated, or -
  group update ID SCOPES NAME | DESC  rename, redescribe or regrant a group
  group public ID yes|no              show a group to anonymous viewers
  group delete ID
  group member ID account:ID|group:N LEVEL|none
  friend request ACCOUNT [at URL for WORLD]   ask an account; a remote one needs its server and world
  friend accept ACCOUNT [at URL for WORLD]    accept a request, likewise for a remote requester
  friend remove ACCOUNT
  receive ENVELOPE_JSON               apply a friend envelope from another server
  verify ACCOUNT                      admin only
  help, quit";

/// Read one line typed at an explorer. Both SSH and the HTML explorer build
/// their actions through [`action_from_fields`], so the two agree on every
/// name and format.
///
/// # Errors
///
/// Fails with a usage message for an unknown command or a wrong argument count.
pub fn parse_line(line: &str) -> Result<Line> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Line::Blank);
    }
    let (word, rest) = split_first(line);
    let words: Vec<&str> = rest.split_whitespace().collect();
    let (action, pairs) = match word {
        "help" | "?" => return Ok(Line::Help),
        "quit" | "exit" => return Ok(Line::Quit),
        "view" => ("view", Vec::new()),
        "signup" => ("sign_up", vec![("name", text_arg(rest, "signup NAME")?)]),
        "keysignup" => (
            "sign_up_key",
            vec![("name", text_arg(rest, "keysignup NAME")?)],
        ),
        "session" => ("open_session", Vec::new()),
        "signout" => ("sign_out", Vec::new()),
        "addkey" => {
            let [key] = words_n(&words, "addkey HEX")?;
            ("add_key", vec![("key", key)])
        }
        "removekey" => {
            let [key] = words_n(&words, "removekey HEX")?;
            ("remove_key", vec![("key", key)])
        }
        "addemail" => {
            let [address] = words_n(&words, "addemail ADDRESS")?;
            ("add_email", vec![("address", address)])
        }
        "removeemail" => {
            let [address] = words_n(&words, "removeemail ADDRESS")?;
            ("remove_email", vec![("address", address)])
        }
        "confirmemail" => {
            let [address, code] = words_n(&words, "confirmemail ADDRESS CODE")?;
            ("confirm_email", vec![("address", address), ("code", code)])
        }
        "signin" => {
            let [address] = words_n(&words, "signin ADDRESS")?;
            ("sign_in_email", vec![("address", address)])
        }
        "confirmsignin" => {
            let [address, code] = words_n(&words, "confirmsignin ADDRESS CODE")?;
            (
                "confirm_sign_in",
                vec![("address", address), ("code", code)],
            )
        }
        "verify" => {
            let [account] = words_n(&words, "verify ACCOUNT")?;
            ("verify", vec![("account", account)])
        }
        "receive" => (
            "receive",
            vec![("envelope", text_arg(rest, "receive ENVELOPE_JSON")?)],
        ),
        "friend" => match split_first(rest) {
            ("request", tail) => {
                let (to, remote) =
                    friend_target(tail, "friend request ACCOUNT [at URL for WORLD]")?;
                ("request_friend", friend_pairs("to", to, remote))
            }
            ("accept", tail) => {
                let (from, remote) =
                    friend_target(tail, "friend accept ACCOUNT [at URL for WORLD]")?;
                ("accept_friend", friend_pairs("from", from, remote))
            }
            ("remove", tail) => {
                let [other] = words_n(&split_words(tail), "friend remove ACCOUNT")?;
                ("remove_friend", vec![("other", other)])
            }
            _ => bail!("friend takes request, accept or remove"),
        },
        "group" => group_command(rest)?,
        other => bail!("unknown command {other}; type help"),
    };
    let mut fields: BTreeMap<String, String> = pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    fields.insert("action".to_owned(), action.to_owned());
    Ok(Line::Action(action_from_fields(&fields)?))
}

/// `ACCOUNT`, or `ACCOUNT at URL for WORLD` for an account on another server.
fn friend_target(tail: &str, usage: &str) -> Result<(String, Option<(String, String)>)> {
    match split_words(tail).as_slice() {
        [account] => Ok(((*account).to_owned(), None)),
        [account, "at", server, "for", world] => Ok((
            (*account).to_owned(),
            Some(((*server).to_owned(), (*world).to_owned())),
        )),
        _ => bail!("usage: {usage}"),
    }
}

fn friend_pairs(
    key: &'static str,
    account: String,
    remote: Option<(String, String)>,
) -> Vec<(&'static str, String)> {
    let mut pairs = vec![(key, account)];
    if let Some((server, audience)) = remote {
        pairs.push(("server", server));
        pairs.push(("audience", audience));
    }
    pairs
}

fn group_command(rest: &str) -> Result<(&'static str, Vec<(&'static str, String)>)> {
    let (sub, tail) = split_first(rest);
    Ok(match sub {
        "new" => {
            let (scopes, name, description) = scoped(tail)?;
            (
                "create_group",
                vec![
                    ("scopes", scopes),
                    ("name", name),
                    ("description", description),
                ],
            )
        }
        "update" => {
            let (id, tail) = split_first(tail);
            let (scopes, name, description) = scoped(tail)?;
            (
                "update_group",
                vec![
                    ("group", id.to_owned()),
                    ("scopes", scopes),
                    ("name", name),
                    ("description", description),
                ],
            )
        }
        "public" => {
            let [group, public] = words_n(&split_words(tail), "group public ID yes|no")?;
            ("set_public", vec![("group", group), ("public", public)])
        }
        "delete" => {
            let [group] = words_n(&split_words(tail), "group delete ID")?;
            ("delete_group", vec![("group", group)])
        }
        "member" => {
            let [group, member, level] = words_n(
                &split_words(tail),
                "group member ID account:ID|group:N LEVEL|none",
            )?;
            (
                "set_member",
                vec![("group", group), ("member", member), ("level", level)],
            )
        }
        _ => bail!("group takes new, update, public, delete or member"),
    })
}

/// `SCOPES NAME | DESCRIPTION`, the description optional.
fn scoped(text: &str) -> Result<(String, String, String)> {
    let (head, description) = text.split_once('|').unwrap_or((text, ""));
    let (scopes, name) = split_first(head);
    if scopes.is_empty() || name.is_empty() {
        bail!("expected SCOPES NAME | DESCRIPTION");
    }
    Ok((
        scopes.to_owned(),
        name.to_owned(),
        description.trim().to_owned(),
    ))
}

fn text_arg(rest: &str, usage: &str) -> Result<String> {
    let text = rest.trim();
    if text.is_empty() {
        bail!("usage: {usage}");
    }
    Ok(text.to_owned())
}

fn words_n<const N: usize>(words: &[&str], usage: &str) -> Result<[String; N]> {
    if words.len() != N {
        bail!("usage: {usage}");
    }
    Ok(std::array::from_fn(|index| words[index].to_owned()))
}

fn split_words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

fn split_first(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.split_once(char::is_whitespace) {
        Some((first, rest)) => (first, rest.trim()),
        None => (text, ""),
    }
}

fn field<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    fields
        .get(key)
        .map(String::as_str)
        .with_context(|| format!("missing {key}"))
}

fn parse_id(text: &str) -> Result<u64> {
    text.parse()
        .with_context(|| format!("{text:?} is not a group ID"))
}

fn parse_bool(text: &str) -> Result<bool> {
    match text {
        "yes" | "on" | "true" => Ok(true),
        "no" | "off" | "false" => Ok(false),
        other => bail!("expected yes or no, not {other:?}"),
    }
}

fn parse_scopes(text: &str) -> Vec<String> {
    if text == "-" {
        return Vec::new();
    }
    text.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_level(text: &str) -> Result<Option<Level>> {
    Ok(Some(match text {
        "" | "none" => return Ok(None),
        "access" => Level::Access,
        "read" => Level::Read,
        "write" => Level::Write,
        "manage" => Level::Manage,
        "moderator" => Level::Moderator,
        "admin" => Level::Admin,
        other => bail!("unknown level {other:?}"),
    }))
}

fn parse_member(text: &str) -> Result<Member> {
    if let Some(id) = text.strip_prefix("account:") {
        return Ok(Member::Account { id: id.to_owned() });
    }
    if let Some(id) = text.strip_prefix("group:") {
        return Ok(Member::Group { id: parse_id(id)? });
    }
    bail!("a member is account:ID or group:N, not {text:?}")
}

/// The server and world a friend action names. Absent or blank fields mean the
/// account is on this server.
fn remote_of(fields: &BTreeMap<String, String>) -> Result<Option<RemoteFriend>> {
    let server = fields
        .get("server")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let audience = fields
        .get("audience")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    match (server, audience) {
        (None, None) => Ok(None),
        (Some(server), Some(audience)) => {
            check_url(server)?;
            Ok(Some(RemoteFriend {
                server: server.to_owned(),
                audience: audience.to_owned(),
            }))
        }
        _ => bail!("a friend on another server needs both its server URL and its world ID"),
    }
}

/// Build an action from named fields. The field `action` names it, in the
/// `snake_case` of the variant, and the rest are the variant's fields. Lists
/// are comma-separated, and `-` is an empty list.
///
/// # Errors
///
/// Fails for an unknown action, a missing field, or a malformed value.
pub fn action_from_fields(fields: &BTreeMap<String, String>) -> Result<DirectoryAction> {
    let action = field(fields, "action")?;
    Ok(match action {
        "view" => DirectoryAction::View,
        "sign_up" => DirectoryAction::SignUp {
            name: field(fields, "name")?.to_owned(),
        },
        "sign_up_key" => DirectoryAction::SignUpKey {
            name: field(fields, "name")?.to_owned(),
        },
        "open_session" => DirectoryAction::OpenSession,
        "sign_out" => DirectoryAction::SignOut,
        "add_key" => DirectoryAction::AddKey {
            key: field(fields, "key")?.to_owned(),
        },
        "remove_key" => DirectoryAction::RemoveKey {
            key: field(fields, "key")?.to_owned(),
        },
        "add_email" => DirectoryAction::AddEmail {
            address: field(fields, "address")?.to_owned(),
        },
        "confirm_email" => DirectoryAction::ConfirmEmail {
            address: field(fields, "address")?.to_owned(),
            code: field(fields, "code")?.to_owned(),
        },
        "remove_email" => DirectoryAction::RemoveEmail {
            address: field(fields, "address")?.to_owned(),
        },
        "sign_in_email" => DirectoryAction::SignInEmail {
            address: field(fields, "address")?.to_owned(),
        },
        "confirm_sign_in" => DirectoryAction::ConfirmSignIn {
            address: field(fields, "address")?.to_owned(),
            code: field(fields, "code")?.to_owned(),
        },
        "create_group" => DirectoryAction::CreateGroup {
            name: field(fields, "name")?.to_owned(),
            description: fields.get("description").cloned().unwrap_or_default(),
            scopes: parse_scopes(field(fields, "scopes")?),
        },
        "update_group" => DirectoryAction::UpdateGroup {
            group: parse_id(field(fields, "group")?)?,
            name: field(fields, "name")?.to_owned(),
            description: fields.get("description").cloned().unwrap_or_default(),
            scopes: parse_scopes(field(fields, "scopes")?),
        },
        "set_public" => DirectoryAction::SetPublic {
            group: parse_id(field(fields, "group")?)?,
            public: parse_bool(field(fields, "public")?)?,
        },
        "delete_group" => DirectoryAction::DeleteGroup {
            group: parse_id(field(fields, "group")?)?,
        },
        "set_member" => DirectoryAction::SetMember {
            group: parse_id(field(fields, "group")?)?,
            member: parse_member(field(fields, "member")?)?,
            level: parse_level(fields.get("level").map_or("", String::as_str))?,
        },
        "request_friend" => DirectoryAction::RequestFriend {
            to: field(fields, "to")?.to_owned(),
            remote: remote_of(fields)?,
        },
        "accept_friend" => DirectoryAction::AcceptFriend {
            from: field(fields, "from")?.to_owned(),
            remote: remote_of(fields)?,
        },
        "remove_friend" => DirectoryAction::RemoveFriend {
            other: field(fields, "other")?.to_owned(),
        },
        "receive" => DirectoryAction::Receive {
            envelope: serde_json::from_str(field(fields, "envelope")?)
                .context("envelope is not valid JSON")?,
        },
        "verify" => DirectoryAction::Verify {
            account: field(fields, "account")?.to_owned(),
        },
        other => bail!("unknown action {other:?}"),
    })
}

/// The result of an action: the view afterwards, and anything the action
/// produced that the caller must hand on.
#[derive(Clone, Serialize)]
pub struct DirectoryOutcome {
    /// What the viewer may see afterwards.
    pub view: DirectoryView,
    /// The new account's credential, set only by sign-up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// Whether a message was handed to the mail sink for this action.
    pub mailed: bool,
    /// A session token, set when a session opens. Shown once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// A message to send. Never serialized: it carries a code.
    #[serde(skip)]
    pub mail: Option<Mail>,
    /// An envelope to queue for another server. Never serialized.
    #[serde(skip)]
    pub outbound: Option<Outbound>,
    /// The change behind `outbound`, not yet applied. The caller queues the
    /// envelope, applies this, and then re-reads the view, which above predates
    /// the change. Never serialized.
    #[serde(skip)]
    pub pending: Option<DirectoryEntry>,
    /// What receiving an envelope did. Never serialized.
    #[serde(skip)]
    pub received: Option<Received>,
}

impl std::fmt::Debug for DirectoryOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryOutcome")
            .field("view", &self.view)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("session", &self.session.as_ref().map(|_| "[REDACTED]"))
            .field("mailed", &self.mailed)
            .field("mail", &self.mail.as_ref().map(|_| "[REDACTED]"))
            .field("outbound", &self.outbound)
            .field("pending", &self.pending)
            .field("received", &self.received)
            .finish()
    }
}

/// Apply an action for a caller and return what that caller may see afterwards.
///
/// # Errors
///
/// Fails with a [`Refusal`] when the caller may not do the action, and with a
/// plain error for input that is malformed.
pub fn run(
    directory: &mut Directory,
    auth: &mut DirectoryAuth,
    caller: &Caller,
    world: &str,
    now: u64,
    action: DirectoryAction,
) -> Result<DirectoryOutcome> {
    let mut viewer = Viewer::resolve(directory, auth, caller, now)?;
    let mut credential = None;
    let mut session = None;
    let mut mail = None;
    let mut outbound = None;
    let mut pending = None;
    let mut received = None;
    match action {
        DirectoryAction::View => {}
        DirectoryAction::SignUp { name } => {
            let (secret, _) = directory.sign_up(&name)?;
            viewer = Viewer::Account(
                directory
                    .account_for(&secret)
                    .context("new account vanished")?,
            );
            credential = Some(secret);
        }
        DirectoryAction::SignUpKey { name } => {
            let key = caller
                .key
                .as_deref()
                .context("signing up with a key needs a request signed with that key")?;
            directory.sign_up_keyed(key, &name)?;
            viewer = Viewer::Account(normalize_key(key)?);
        }
        DirectoryAction::OpenSession => {
            let principal = match &viewer {
                Viewer::Admin => Principal::Admin,
                Viewer::Account(id) => Principal::Account(id.clone()),
                Viewer::Anonymous => return Err(unauthenticated("sign in first")),
            };
            session = Some(auth.open_session(principal, now));
        }
        DirectoryAction::SignOut => {
            if let Some(token) = &caller.session {
                auth.close_session(token);
            }
            viewer = Viewer::Anonymous;
        }
        DirectoryAction::AddKey { key } => {
            let me = signed_in(&viewer)?;
            let key = normalize_key(&key)?;
            if caller.key.as_deref() != Some(key.as_str()) {
                return Err(Refusal::Forbidden(
                    "sign the request with the key you are adding".to_owned(),
                )
                .into());
            }
            directory.add_key(Actor::Account(me), me, &key)?;
        }
        DirectoryAction::RemoveKey { key } => {
            let me = signed_in(&viewer)?;
            directory.remove_key(Actor::Account(me), me, &key)?;
        }
        DirectoryAction::AddEmail { address } => {
            let me = signed_in(&viewer)?;
            let address = normalize_email(&address)?;
            if directory.account_for_email(&address).is_some() {
                return Err(Refusal::Conflict(
                    "that address already belongs to an account".to_owned(),
                )
                .into());
            }
            let code = auth.issue_code(
                Purpose::Confirm {
                    account: me.to_owned(),
                },
                &address,
                now,
            )?;
            mail = Some(code_mail(&address, &code, "confirm your address"));
        }
        DirectoryAction::ConfirmEmail { address, code } => {
            let me = signed_in(&viewer)?;
            let address = normalize_email(&address)?;
            match auth.check_code(&address, &code, now)? {
                Purpose::Confirm { account } if account == me => {}
                _ => return Err(unauthenticated("no matching code")),
            }
            directory.confirm_email(Actor::Server, me, &address)?;
        }
        DirectoryAction::RemoveEmail { address } => {
            let me = signed_in(&viewer)?;
            directory.remove_email(Actor::Account(me), me, &address)?;
        }
        DirectoryAction::SignInEmail { address } => {
            let address = normalize_email(&address)?;
            if let Some(account) = directory.account_for_email(&address)
                && let Ok(code) = auth.issue_code(Purpose::SignIn { account }, &address, now)
            {
                mail = Some(code_mail(&address, &code, "sign-in code"));
            }
        }
        DirectoryAction::ConfirmSignIn { address, code } => {
            let address = normalize_email(&address)?;
            match auth.check_code(&address, &code, now)? {
                Purpose::SignIn { account } => {
                    session = Some(auth.open_session(Principal::Account(account.clone()), now));
                    viewer = Viewer::Account(account);
                }
                Purpose::Confirm { .. } => return Err(unauthenticated("no matching code")),
            }
        }
        DirectoryAction::CreateGroup {
            name,
            description,
            scopes,
        } => {
            let me = signed_in(&viewer)?;
            let scopes = scopes_of(&scopes)?;
            within_ceiling(directory, Actor::Account(me), scopes)?;
            directory.create_group(me, &name, &description, scopes)?;
        }
        DirectoryAction::UpdateGroup {
            group,
            name,
            description,
            scopes,
        } => {
            let actor = actor_of(&viewer)?;
            let scopes = scopes_of(&scopes)?;
            within_ceiling(directory, actor, scopes)?;
            directory.update_group(actor, group, &name, &description, scopes)?;
        }
        DirectoryAction::SetPublic { group, public } => {
            directory.set_public(actor_of(&viewer)?, group, public)?;
        }
        DirectoryAction::DeleteGroup { group } => {
            directory.delete_group(actor_of(&viewer)?, group)?;
        }
        DirectoryAction::SetMember {
            group,
            member,
            level,
        } => {
            directory.set_member(actor_of(&viewer)?, group, member, level)?;
        }
        DirectoryAction::RequestFriend { to, remote } => {
            let me = signed_in(&viewer)?;
            match remote {
                None => {
                    directory.request_friend_as(me, &to)?;
                }
                Some(remote) => {
                    let (envelope, entry) = directory.prepare_request_friend(
                        sender_credential(caller)?,
                        &to,
                        &remote.audience,
                        now,
                    )?;
                    outbound = Some(outbound_to(&remote, envelope));
                    pending = Some(entry);
                }
            }
        }
        DirectoryAction::AcceptFriend { from, remote } => {
            let me = signed_in(&viewer)?;
            match remote {
                None => {
                    directory.accept_friend_as(me, &from)?;
                }
                Some(remote) => {
                    let (envelope, entry) = directory.prepare_accept_friend(
                        sender_credential(caller)?,
                        &from,
                        &remote.audience,
                        now,
                    )?;
                    outbound = Some(outbound_to(&remote, envelope));
                    pending = Some(entry);
                }
            }
        }
        DirectoryAction::RemoveFriend { other } => {
            let me = signed_in(&viewer)?;
            directory.remove_friend_as(me, &other)?;
        }
        DirectoryAction::Receive { envelope } => {
            received = Some(directory.receive(&envelope, world, now)?);
        }
        DirectoryAction::Verify { account } => {
            directory.verify_account(actor_of(&viewer)?, &account)?;
        }
    }
    Ok(DirectoryOutcome {
        view: view(directory, &viewer),
        credential,
        session,
        mailed: false,
        mail,
        outbound,
        pending,
        received,
    })
}

fn unauthenticated(message: &str) -> anyhow::Error {
    anyhow!(Refusal::Unauthenticated(message.to_owned()))
}

fn sender_credential(caller: &Caller) -> Result<&str> {
    caller.credential.as_deref().ok_or_else(|| {
        unauthenticated("sending to an account on another server needs your credential")
    })
}

fn outbound_to(remote: &RemoteFriend, envelope: Envelope) -> Outbound {
    Outbound {
        url: format!("{}/envelope", remote.server.trim_end_matches('/')),
        envelope,
    }
}

fn signed_in(viewer: &Viewer) -> Result<&str> {
    match viewer {
        Viewer::Account(id) => Ok(id),
        Viewer::Anonymous | Viewer::Admin => Err(unauthenticated("sign in as an account first")),
    }
}

fn actor_of(viewer: &Viewer) -> Result<Actor<'_>> {
    match viewer {
        Viewer::Admin => Ok(Actor::Server),
        Viewer::Account(id) => Ok(Actor::Account(id)),
        Viewer::Anonymous => Err(unauthenticated("sign in first")),
    }
}

fn scopes_of(names: &[String]) -> Result<WorldScopes> {
    WorldScopes::from_names(names.iter().map(String::as_str)).context("unknown permission name")
}

/// An account may only grant permissions it holds itself; the admin is not held to this.
fn within_ceiling(directory: &Directory, actor: Actor<'_>, scopes: WorldScopes) -> Result<()> {
    if let Actor::Account(id) = actor
        && !directory.ceiling(id).contains(scopes)
    {
        return Err(Refusal::Forbidden(
            "a group cannot grant permissions you do not hold".to_owned(),
        )
        .into());
    }
    Ok(())
}

fn code_mail(address: &str, code: &str, subject: &str) -> Mail {
    Mail {
        to: address.to_owned(),
        subject: format!("id: {subject}"),
        body: format!("Your code is {code}. It works for 15 minutes.\n"),
    }
}

fn view(directory: &Directory, viewer: &Viewer) -> DirectoryView {
    let (label, me, admin) = match viewer {
        Viewer::Anonymous => ("anonymous".to_owned(), None, false),
        Viewer::Account(id) => (id.clone(), Some(id.as_str()), false),
        Viewer::Admin => ("admin".to_owned(), None, true),
    };
    let accounts = directory
        .accounts()
        .map(|account| {
            let own = admin || me == Some(account.id.as_str());
            let pending = own.then(|| directory.pending_for(&account.id));
            AccountView {
                id: account.id.clone(),
                name: account.name.clone(),
                verified: own.then(|| directory.is_verified(&account.id)),
                keys: own.then(|| account.keys.iter().cloned().collect()),
                emails: own.then(|| account.emails.iter().cloned().collect()),
                scopes: own.then(|| scope_names(directory.ceiling(&account.id))),
                friends: own.then(|| directory.friends_of(&account.id)),
                incoming: pending.as_ref().map(|(incoming, _)| incoming.clone()),
                outgoing: pending.map(|(_, outgoing)| outgoing),
            }
        })
        .collect();
    let groups = directory
        .groups()
        .filter_map(|group| {
            let level = me.and_then(|id| directory.level_in(id, group.id));
            if !(admin || group.public || level.is_some()) {
                return None;
            }
            let detail = admin || level.is_some_and(|level| level >= Level::Read);
            Some(GroupView {
                id: group.id,
                name: group.name.clone(),
                description: group.description.clone(),
                public: group.public,
                scopes: scope_names(group.scopes),
                level,
                members: detail.then(|| members(directory, group.id)),
            })
        })
        .collect();
    DirectoryView {
        viewer: label,
        accounts,
        groups,
        deliveries: Vec::new(),
    }
}

fn members(directory: &Directory, group: u64) -> Vec<MemberView> {
    directory
        .members_of(group)
        .into_iter()
        .map(|(member, level)| {
            let name = match &member {
                Member::Account { id } => directory
                    .account(id)
                    .map_or_else(|| id.clone(), |account| account.name.clone()),
                Member::Group { id } => directory
                    .group(*id)
                    .map_or_else(|| id.to_string(), |group| group.name.clone()),
            };
            MemberView {
                member,
                name,
                level,
            }
        })
        .collect()
}

fn scope_names(scopes: WorldScopes) -> Vec<String> {
    [
        (WorldScopes::JOIN, "join"),
        (WorldScopes::CHAT, "chat"),
        (WorldScopes::INPUT, "input"),
        (WorldScopes::DELEGATE, "delegate"),
    ]
    .into_iter()
    .filter(|(bit, _)| scopes.contains(*bit))
    .map(|(_, name)| name.to_owned())
    .collect()
}

pub(crate) const fn level_name(level: Level) -> &'static str {
    match level {
        Level::Access => "access",
        Level::Read => "read",
        Level::Write => "write",
        Level::Manage => "manage",
        Level::Moderator => "moderator",
        Level::Admin => "admin",
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.join(" ")
    }
}

/// The view as plain text, for the SSH explorer.
#[must_use]
pub fn render_text(view: &DirectoryView) -> String {
    let mut lines = vec![
        format!("viewer: {}", view.viewer),
        String::new(),
        "accounts:".to_owned(),
    ];
    if view.accounts.is_empty() {
        lines.push("  (none)".to_owned());
    }
    for account in &view.accounts {
        lines.push(format!("  {}  {}", account.id, account.name));
        let mut details = Vec::new();
        if let Some(verified) = account.verified {
            details.push(format!("verified: {}", if verified { "yes" } else { "no" }));
        }
        if let Some(keys) = &account.keys {
            details.push(format!("keys: {}", list(keys)));
        }
        if let Some(emails) = &account.emails {
            details.push(format!("emails: {}", list(emails)));
        }
        if let Some(scopes) = &account.scopes {
            details.push(format!("scopes: {}", list(scopes)));
        }
        if let Some(friends) = &account.friends {
            details.push(format!("friends: {}", list(friends)));
        }
        if let Some(incoming) = &account.incoming {
            details.push(format!("requests in: {}", list(incoming)));
        }
        if let Some(outgoing) = &account.outgoing {
            details.push(format!("requests out: {}", list(outgoing)));
        }
        if !details.is_empty() {
            lines.push(format!("      {}", details.join("  ")));
        }
    }
    lines.push(String::new());
    lines.push("groups:".to_owned());
    if view.groups.is_empty() {
        lines.push("  (none)".to_owned());
    }
    for group in &view.groups {
        let visibility = if group.public { "public" } else { "private" };
        let level = group.level.map_or("-", level_name);
        lines.push(format!(
            "  {}  {}  {visibility}  you: {level}  scopes: {}",
            group.id,
            group.name,
            list(&group.scopes)
        ));
        if !group.description.is_empty() {
            lines.push(format!("      {}", group.description));
        }
        for member in group.members.iter().flatten() {
            let (kind, id) = match &member.member {
                Member::Account { id } => ("account", id.clone()),
                Member::Group { id } => ("group", id.to_string()),
            };
            lines.push(format!(
                "      {kind} {id}  {}  {}",
                member.name,
                level_name(member.level)
            ));
        }
    }
    if !view.deliveries.is_empty() {
        lines.push(String::new());
        lines.push("deliveries:".to_owned());
        for delivery in &view.deliveries {
            lines.push(format!(
                "  {}  {}  {}",
                delivery.kind, delivery.to, delivery.status
            ));
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn sign(directory: &mut Directory, name: &str) -> (String, String) {
        let (credential, _) = directory.sign_up(name).unwrap();
        let id = directory.account_for(&credential).unwrap();
        (credential, id)
    }

    struct Sample {
        directory: Directory,
        ada: String,
        ada_credential: String,
        bo: String,
        cy: String,
        club: u64,
        open: u64,
    }

    fn sample() -> Sample {
        let mut directory = Directory::new();
        let (ada_credential, ada) = sign(&mut directory, "Ada");
        let (_, bo) = sign(&mut directory, "Bo");
        let (_, cy) = sign(&mut directory, "Cy");
        directory.verify_account(Actor::Server, &ada).unwrap();
        let (club, _) = directory
            .create_group(&ada, "Club", "Members only", WorldScopes::GUEST)
            .unwrap();
        let (open, _) = directory
            .create_group(&ada, "Open", "Anyone may look", WorldScopes::GUEST)
            .unwrap();
        directory
            .set_public(Actor::Account(&ada), open, true)
            .unwrap();
        directory
            .set_member(
                Actor::Account(&ada),
                club,
                Member::Account { id: bo.clone() },
                Some(Level::Read),
            )
            .unwrap();
        directory
            .request_friend(&ada_credential, &cy, "club", 1)
            .unwrap();
        Sample {
            directory,
            ada,
            ada_credential,
            bo,
            cy,
            club,
            open,
        }
    }

    fn view_for(directory: &Directory, viewer: &Viewer) -> DirectoryView {
        view(directory, viewer)
    }

    fn act(
        directory: &mut Directory,
        caller: &Caller,
        action: DirectoryAction,
    ) -> Result<DirectoryOutcome> {
        run(
            directory,
            &mut DirectoryAuth::default(),
            caller,
            "test-world",
            0,
            action,
        )
    }

    #[test]
    fn a_remote_request_is_queued_for_its_server_and_received_there() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann_credential, ann) = sign(&mut home, "Ann");
        let (_, bo) = sign(&mut away, "Bo");
        let remote = DirectoryAction::RequestFriend {
            to: bo.clone(),
            remote: Some(RemoteFriend {
                server: "https://away.example/".to_owned(),
                audience: "lobby".to_owned(),
            }),
        };
        let refused = act(&mut home, &Caller::default(), remote.clone()).unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<Refusal>(),
            Some(Refusal::Unauthenticated(_))
        ));
        let caller = Caller {
            credential: Some(ann_credential),
            ..Caller::default()
        };
        let outbound = act(&mut home, &caller, remote).unwrap().outbound.unwrap();
        assert_eq!(outbound.url, "https://away.example/envelope");
        assert_eq!(outbound.envelope.audience, "lobby");
        assert_eq!(outbound.envelope.to, bo);
        assert!(matches!(
            away.receive(&outbound.envelope, "lobby", 0).unwrap(),
            Received::Applied(_)
        ));
        assert_eq!(away.pending_for(&bo).0, vec![ann]);
    }

    #[test]
    fn a_queued_remote_request_shows_until_it_is_delivered_or_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann_credential, ann) = sign(&mut home, "Ann");
        let (_, bo) = sign(&mut away, "Bo");
        let caller = Caller {
            credential: Some(ann_credential),
            ..Caller::default()
        };
        let outbound = act(
            &mut home,
            &caller,
            DirectoryAction::RequestFriend {
                to: bo.clone(),
                remote: Some(RemoteFriend {
                    server: "https://away.example".to_owned(),
                    audience: "lobby".to_owned(),
                }),
            },
        )
        .unwrap()
        .outbound
        .unwrap();
        let id = outbound.envelope.id.clone();
        let mut outbox = Outbox::open(dir.path().join("outbox.jsonl")).unwrap();
        outbox.enqueue(&outbound.url, outbound.envelope).unwrap();

        let shown = deliveries(&outbox, &ann);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].to, bo);
        assert_eq!(shown[0].status, "not yet delivered");
        assert!(deliveries(&outbox, &bo).is_empty());
        assert!(deliveries(&outbox, "anonymous").is_empty());

        outbox
            .record(
                &id,
                0,
                crate::envelope_outbox::Outcome::Refused("no such account".to_owned()),
            )
            .unwrap();
        assert_eq!(
            deliveries(&outbox, &ann)[0].status,
            "refused: no such account"
        );
    }

    #[test]
    fn the_admin_sees_every_detail_and_member() {
        let s = sample();
        let view = view_for(&s.directory, &Viewer::Admin);
        assert_eq!(view.accounts.len(), 3);
        assert!(view.accounts.iter().all(|a| a.verified.is_some()));
        let ada = view.accounts.iter().find(|a| a.id == s.ada).unwrap();
        assert_eq!(ada.friends.as_deref(), Some(&[][..]));
        assert_eq!(ada.outgoing.as_deref(), Some(&[s.cy.clone()][..]));
        assert_eq!(view.groups.len(), 2);
        let club = view.groups.iter().find(|g| g.id == s.club).unwrap();
        assert_eq!(club.members.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn anonymous_sees_names_and_public_groups_only() {
        let s = sample();
        let view = view_for(&s.directory, &Viewer::Anonymous);
        assert_eq!(view.accounts.len(), 3);
        assert!(view.accounts.iter().all(|a| {
            a.verified.is_none()
                && a.scopes.is_none()
                && a.friends.is_none()
                && a.incoming.is_none()
        }));
        assert_eq!(view.groups.len(), 1);
        assert_eq!(view.groups[0].id, s.open);
        assert!(view.groups[0].members.is_none());
        let text = render_text(&view);
        assert!(!text.contains("Club"));
        assert!(!text.contains("friends:"));
    }

    #[test]
    fn a_member_sees_its_groups_and_members_at_read_but_not_others() {
        let s = sample();
        let view = view_for(&s.directory, &Viewer::Account(s.bo.clone()));
        let club = view.groups.iter().find(|g| g.id == s.club).unwrap();
        assert_eq!(club.level, Some(Level::Read));
        assert_eq!(club.members.as_ref().unwrap().len(), 2);
        let bo = view.accounts.iter().find(|a| a.id == s.bo).unwrap();
        assert_eq!(bo.verified, Some(false));
        assert_eq!(bo.friends.as_deref(), Some(&[][..]));
        let ada = view.accounts.iter().find(|a| a.id == s.ada).unwrap();
        assert!(ada.friends.is_none());
        assert!(ada.verified.is_none());
    }

    #[test]
    fn an_access_member_sees_the_group_without_its_members() {
        let mut s = sample();
        s.directory
            .set_member(
                Actor::Account(&s.ada),
                s.club,
                Member::Account { id: s.cy.clone() },
                Some(Level::Access),
            )
            .unwrap();
        let view = view_for(&s.directory, &Viewer::Account(s.cy.clone()));
        let club = view.groups.iter().find(|g| g.id == s.club).unwrap();
        assert_eq!(club.level, Some(Level::Access));
        assert!(club.members.is_none());
    }

    #[test]
    fn friend_requests_show_on_both_ends() {
        let s = sample();
        let cy = view_for(&s.directory, &Viewer::Account(s.cy.clone()));
        let cy_details = cy.accounts.iter().find(|a| a.id == s.cy).unwrap();
        assert_eq!(cy_details.incoming.as_deref(), Some(&[s.ada.clone()][..]));
        let ada = view_for(&s.directory, &Viewer::Account(s.ada.clone()));
        let ada_details = ada.accounts.iter().find(|a| a.id == s.ada).unwrap();
        assert_eq!(ada_details.outgoing.as_deref(), Some(&[s.cy.clone()][..]));
    }

    #[test]
    fn sign_up_returns_a_credential_and_the_new_account_view() {
        let mut s = sample();
        let outcome = act(
            &mut s.directory,
            &Caller::default(),
            DirectoryAction::SignUp {
                name: "Dee".to_owned(),
            },
        )
        .unwrap();
        let credential = outcome.credential.clone().unwrap();
        let dee = s.directory.account_for(&credential).unwrap();
        assert_eq!(outcome.view.viewer, dee);
        let details = outcome.view.accounts.iter().find(|a| a.id == dee).unwrap();
        assert_eq!(details.verified, Some(false));
        assert_eq!(outcome.view.accounts.len(), 4);
    }

    #[test]
    fn a_credential_naming_no_account_is_refused_unless_admin() {
        let s = sample();
        let mut auth = DirectoryAuth::default();
        let unknown = Caller::from_secret("acct.nobody.00");
        let refused = Viewer::resolve(&s.directory, &mut auth, &unknown, 0).unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<Refusal>(),
            Some(Refusal::Unauthenticated(_))
        ));
        let admin = Caller {
            admin: true,
            ..unknown
        };
        assert_eq!(
            Viewer::resolve(&s.directory, &mut auth, &admin, 0).unwrap(),
            Viewer::Admin
        );
        assert_eq!(
            Viewer::resolve(
                &s.directory,
                &mut auth,
                &Caller::from_secret(&s.ada_credential),
                0
            )
            .unwrap(),
            Viewer::Account(s.ada.clone())
        );
        assert_eq!(
            Viewer::resolve(&s.directory, &mut auth, &Caller::default(), 0).unwrap(),
            Viewer::Anonymous
        );
    }

    #[test]
    fn a_keyed_account_adds_keys_and_emails_and_signs_in_by_email() {
        let mut s = sample();
        let key =
            crate::world::hex_encode(&SigningKey::from_bytes(&[5; 32]).verifying_key().to_bytes());
        let signed = Caller {
            key: Some(key.clone()),
            ..Caller::default()
        };
        let mut auth = DirectoryAuth::default();
        let outcome = run(
            &mut s.directory,
            &mut auth,
            &signed,
            "test-world",
            0,
            DirectoryAction::SignUpKey {
                name: "Kay".to_owned(),
            },
        )
        .unwrap();
        assert!(outcome.credential.is_none() && outcome.session.is_none());
        assert_eq!(outcome.view.viewer, key);

        let signed_in = Caller {
            key: Some(key.clone()),
            ..Caller::default()
        };
        let other =
            crate::world::hex_encode(&SigningKey::from_bytes(&[6; 32]).verifying_key().to_bytes());
        let refused = run(
            &mut s.directory,
            &mut auth,
            &signed_in,
            "test-world",
            0,
            DirectoryAction::AddKey { key: other },
        )
        .unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<Refusal>(),
            Some(Refusal::Forbidden(_))
        ));

        let address = "kay@example.com";
        let asked = run(
            &mut s.directory,
            &mut auth,
            &signed_in,
            "test-world",
            0,
            DirectoryAction::AddEmail {
                address: address.to_owned(),
            },
        )
        .unwrap();
        let mail = asked.mail.clone().unwrap();
        assert_eq!(mail.to, address);
        let code = mail
            .body
            .split_whitespace()
            .nth(3)
            .unwrap()
            .trim_end_matches('.')
            .to_owned();
        assert!(!format!("{asked:?}").contains(&code));

        run(
            &mut s.directory,
            &mut auth,
            &signed_in,
            "test-world",
            0,
            DirectoryAction::ConfirmEmail {
                address: address.to_owned(),
                code,
            },
        )
        .unwrap();
        let shown = view(&s.directory, &Viewer::Account(key.clone()));
        let me = shown.accounts.iter().find(|a| a.id == key).unwrap();
        assert_eq!(me.emails.as_deref(), Some(&[address.to_owned()][..]));
        assert_eq!(me.verified, Some(true));

        let sign_in = run(
            &mut s.directory,
            &mut auth,
            &Caller::default(),
            "test-world",
            0,
            DirectoryAction::SignInEmail {
                address: address.to_owned(),
            },
        )
        .unwrap();
        assert_eq!(sign_in.mail.unwrap().to, address);
        let missing = run(
            &mut s.directory,
            &mut auth,
            &Caller::default(),
            "test-world",
            0,
            DirectoryAction::SignInEmail {
                address: "nobody@example.com".to_owned(),
            },
        )
        .unwrap();
        assert!(missing.mail.is_none());
    }

    #[test]
    fn only_a_group_admin_makes_a_group_public() {
        let mut s = sample();
        s.directory
            .set_member(
                Actor::Account(&s.ada),
                s.club,
                Member::Account { id: s.bo.clone() },
                Some(Level::Manage),
            )
            .unwrap();
        assert!(
            s.directory
                .set_public(Actor::Account(&s.bo), s.club, true)
                .is_err()
        );
        s.directory
            .set_public(Actor::Account(&s.ada), s.club, true)
            .unwrap();
        let view = view_for(&s.directory, &Viewer::Anonymous);
        let club = view.groups.iter().find(|g| g.id == s.club).unwrap();
        assert!(club.public && club.members.is_none());
    }

    #[test]
    fn json_and_text_name_the_same_accounts_and_groups() {
        let s = sample();
        let viewers = [
            Viewer::Admin,
            Viewer::Anonymous,
            Viewer::Account(s.bo.clone()),
        ];
        for viewer in viewers {
            let shown = view(&s.directory, &viewer);
            let json = serde_json::to_string(&shown).unwrap();
            let text = render_text(&shown);
            for account in &shown.accounts {
                assert!(json.contains(&account.id) && text.contains(&account.id));
                assert!(json.contains(&account.name) && text.contains(&account.name));
            }
            for group in &shown.groups {
                assert!(json.contains(&group.name) && text.contains(&group.name));
            }
        }
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_typed_line_becomes_the_action_it_names() {
        assert_eq!(parse_line("   ").unwrap(), Line::Blank);
        assert_eq!(parse_line("help").unwrap(), Line::Help);
        assert_eq!(parse_line("quit").unwrap(), Line::Quit);
        assert_eq!(parse_line("exit").unwrap(), Line::Quit);
        assert_eq!(
            parse_line("signup Cy").unwrap(),
            Line::Action(DirectoryAction::SignUp {
                name: "Cy".to_owned()
            })
        );
        assert_eq!(
            parse_line("group new - Club | Members only").unwrap(),
            Line::Action(DirectoryAction::CreateGroup {
                name: "Club".to_owned(),
                description: "Members only".to_owned(),
                scopes: Vec::new(),
            })
        );
        assert_eq!(
            parse_line("group new join,chat Club | Members only").unwrap(),
            Line::Action(DirectoryAction::CreateGroup {
                name: "Club".to_owned(),
                description: "Members only".to_owned(),
                scopes: vec!["join".to_owned(), "chat".to_owned()],
            })
        );
        assert_eq!(
            parse_line("group member 3 account:abc read").unwrap(),
            Line::Action(DirectoryAction::SetMember {
                group: 3,
                member: Member::Account {
                    id: "abc".to_owned()
                },
                level: Some(Level::Read),
            })
        );
        assert_eq!(
            parse_line("group member 3 group:4 none").unwrap(),
            Line::Action(DirectoryAction::SetMember {
                group: 3,
                member: Member::Group { id: 4 },
                level: None,
            })
        );
        assert_eq!(
            parse_line("friend request abc").unwrap(),
            Line::Action(DirectoryAction::RequestFriend {
                to: "abc".to_owned(),
                remote: None,
            })
        );
        assert_eq!(
            parse_line("friend accept abc at https://example.com for lobby").unwrap(),
            Line::Action(DirectoryAction::AcceptFriend {
                from: "abc".to_owned(),
                remote: Some(RemoteFriend {
                    server: "https://example.com".to_owned(),
                    audience: "lobby".to_owned(),
                }),
            })
        );
    }

    #[test]
    fn a_malformed_line_is_refused_with_its_usage() {
        for line in [
            "bogus",
            "signup",
            "group new",
            "group member 3 account:x wizard",
            "group member 3 pigeon read",
            "friend request",
        ] {
            assert!(parse_line(line).is_err(), "{line:?} should be refused");
        }
    }

    #[test]
    fn form_fields_build_the_same_action_as_the_typed_line() {
        assert_eq!(
            action_from_fields(&fields(&[("action", "view")])).unwrap(),
            DirectoryAction::View
        );
        assert_eq!(
            action_from_fields(&fields(&[("action", "sign_up"), ("name", "Cy")])).unwrap(),
            DirectoryAction::SignUp {
                name: "Cy".to_owned()
            }
        );
        assert_eq!(
            action_from_fields(&fields(&[
                ("action", "set_member"),
                ("group", "3"),
                ("member", "account:abc"),
                ("level", ""),
            ]))
            .unwrap(),
            DirectoryAction::SetMember {
                group: 3,
                member: Member::Account {
                    id: "abc".to_owned()
                },
                level: None,
            }
        );
        assert!(action_from_fields(&fields(&[("action", "explode")])).is_err());
        assert!(action_from_fields(&fields(&[("name", "Cy")])).is_err());
    }
}
