//! What a viewer may see of a server's directory, shaped once for every transport.
//!
//! HTTP serializes a [`DirectoryView`] as JSON and SSH renders it with
//! [`render_text`]. Visibility is decided here alone, so an entity shown on one
//! transport is shown on the other.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::directory::{Directory, Level, Member};
use crate::world::WorldScopes;

/// Who is looking at the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    /// No credential: account names and public groups only.
    Anonymous,
    /// An account, by its ID.
    Account(String),
    /// The admin token: everything.
    Admin,
}

impl Viewer {
    /// The viewer a request presents. The admin token wins over a credential.
    ///
    /// # Errors
    ///
    /// Fails when the credential names no account.
    pub fn resolve(directory: &Directory, admin: bool, credential: Option<&str>) -> Result<Self> {
        if admin {
            return Ok(Self::Admin);
        }
        match credential {
            None => Ok(Self::Anonymous),
            Some(credential) => directory
                .account_for(credential)
                .map(Self::Account)
                .context("unknown credential"),
        }
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
}

/// An account as one viewer sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountView {
    /// The account ID.
    pub id: String,
    /// The display name.
    pub name: String,
    /// Whether the account is verified; shown to the account and the admin.
    pub verified: Option<bool>,
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
    /// Create an account. The credential is returned once, and the view becomes the new account's.
    SignUp {
        /// The display name.
        name: String,
    },
}

/// The result of an action: the view afterwards, and a new credential on sign-up.
#[derive(Clone, Serialize)]
pub struct DirectoryOutcome {
    /// What the viewer may see afterwards.
    pub view: DirectoryView,
    /// The new account's credential, set only by sign-up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

impl std::fmt::Debug for DirectoryOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryOutcome")
            .field("view", &self.view)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Apply an action for a viewer and return what that viewer may see afterwards.
///
/// # Errors
///
/// Fails when the action is refused, such as a sign-up with an invalid name.
pub fn run(
    directory: &mut Directory,
    viewer: &Viewer,
    action: DirectoryAction,
) -> Result<DirectoryOutcome> {
    match action {
        DirectoryAction::View => Ok(DirectoryOutcome {
            view: view(directory, viewer),
            credential: None,
        }),
        DirectoryAction::SignUp { name } => {
            let (credential, _) = directory.sign_up(&name)?;
            let id = directory
                .account_for(&credential)
                .context("new account vanished")?;
            Ok(DirectoryOutcome {
                view: view(directory, &Viewer::Account(id)),
                credential: Some(credential),
            })
        }
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
                verified: own.then_some(account.verified),
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

const fn level_name(level: Level) -> &'static str {
    match level {
        Level::Access => "access",
        Level::Read => "read",
        Level::Write => "write",
        Level::Manage => "manage",
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
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::directory::Actor;

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
        directory.request_friend(&ada_credential, &cy, 1).unwrap();
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

    fn view_for(directory: &mut Directory, viewer: &Viewer) -> DirectoryView {
        run(directory, viewer, DirectoryAction::View).unwrap().view
    }

    #[test]
    fn the_admin_sees_every_detail_and_member() {
        let mut s = sample();
        let view = view_for(&mut s.directory, &Viewer::Admin);
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
        let mut s = sample();
        let view = view_for(&mut s.directory, &Viewer::Anonymous);
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
        let mut s = sample();
        let view = view_for(&mut s.directory, &Viewer::Account(s.bo.clone()));
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
        let view = view_for(&mut s.directory, &Viewer::Account(s.cy.clone()));
        let club = view.groups.iter().find(|g| g.id == s.club).unwrap();
        assert_eq!(club.level, Some(Level::Access));
        assert!(club.members.is_none());
    }

    #[test]
    fn friend_requests_show_on_both_ends() {
        let mut s = sample();
        let cy = view_for(&mut s.directory, &Viewer::Account(s.cy.clone()));
        let cy_details = cy.accounts.iter().find(|a| a.id == s.cy).unwrap();
        assert_eq!(cy_details.incoming.as_deref(), Some(&[s.ada.clone()][..]));
        let ada = view_for(&mut s.directory, &Viewer::Account(s.ada.clone()));
        let ada_details = ada.accounts.iter().find(|a| a.id == s.ada).unwrap();
        assert_eq!(ada_details.outgoing.as_deref(), Some(&[s.cy.clone()][..]));
    }

    #[test]
    fn sign_up_returns_a_credential_and_the_new_account_view() {
        let mut s = sample();
        let outcome = run(
            &mut s.directory,
            &Viewer::Anonymous,
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
        assert!(Viewer::resolve(&s.directory, false, Some("acct.nobody.00")).is_err());
        assert_eq!(
            Viewer::resolve(&s.directory, true, Some("acct.nobody.00")).unwrap(),
            Viewer::Admin
        );
        assert_eq!(
            Viewer::resolve(&s.directory, false, Some(&s.ada_credential)).unwrap(),
            Viewer::Account(s.ada.clone())
        );
        assert_eq!(
            Viewer::resolve(&s.directory, false, None).unwrap(),
            Viewer::Anonymous
        );
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
        let view = view_for(&mut s.directory, &Viewer::Anonymous);
        let club = view.groups.iter().find(|g| g.id == s.club).unwrap();
        assert!(club.public && club.members.is_none());
    }

    #[test]
    fn json_and_text_name_the_same_accounts_and_groups() {
        let mut s = sample();
        let viewers = [
            Viewer::Admin,
            Viewer::Anonymous,
            Viewer::Account(s.bo.clone()),
        ];
        for viewer in viewers {
            let outcome = run(&mut s.directory, &viewer, DirectoryAction::View).unwrap();
            let json = serde_json::to_string(&outcome.view).unwrap();
            let text = render_text(&outcome.view);
            for account in &outcome.view.accounts {
                assert!(json.contains(&account.id) && text.contains(&account.id));
                assert!(json.contains(&account.name) && text.contains(&account.name));
            }
            for group in &outcome.view.groups {
                assert!(json.contains(&group.name) && text.contains(&group.name));
            }
        }
    }
}
