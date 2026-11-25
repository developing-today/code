//! Shared work surfaces: task boards and shared documents as world
//! state — v3 shared-agent-worlds step 1 (#10049, parent #6302).
//!
//! Oracle: `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
//! §2.1–§2.3 and §4 decisions 1–2, over the v2 role configuration
//! ([`crate::role_config`], #8299) and multi-agent sessions
//! ([`crate::world_agents`], #8300).
//!
//! # The model
//!
//! - A **board** is world configuration, like a role: its states (the
//!   first is where new items start; some are closing states), which
//!   roles may read, create, claim, move and close, and the claim lease.
//!   Defining or editing one bumps its version.
//! - An **item** carries title, description, state, creator, priority,
//!   blocked-by, artifacts, its current claim and a version that every
//!   change bumps. Its full history is the audit log.
//! - A **claim** is a checked write: it names the item version the
//!   claimer saw, and fails as stale if the item moved since, or as
//!   contended if another live claim holds it — never a silent double
//!   assignment. A claim lasts the board's lease; an expired claim
//!   returns the item to the state it had when claimed, as an audited
//!   world event, so a parked agent cannot strand work.
//! - A **shared document** is world state too: content, a revision, a
//!   review state, and a history in which every edit is attributed to
//!   (agent, role) with its payload hash. Its conflict model is
//!   configuration: append-only, or last-writer-wins with the replaced
//!   text kept in history. Neither permits an unrecorded overwrite, so
//!   any past revision can be rebuilt ([`WorkSurfaces::document_at`]).
//!
//! # Authority
//!
//! Every act names its actor. An agent acts under a role, and the act is
//! refused unless the world's [`RoleRegistry`] holds that grant and the
//! surface's configuration gives that role the permission. The owner
//! acts on everything. No act on a surface widens anyone's ceiling: a
//! claim records a holder, it delegates nothing. Every mutation, and
//! every refused act including refused reads, is an audit event
//! ([`SurfaceEvent`]) — contention is data, not an error path.
//!
//! State is keyed by agent key, never by session, so it survives an
//! agent leaving and rejoining.
//!
//! # Learning that a surface changed
//!
//! Each accepted change appends a semantic diff ([`SurfaceChange`]) under
//! a world-wide revision. An agent subscribed to a surface asks for
//! everything since the revision it last saw ([`WorkSurfaces::catch_up`]).
//! A host that still holds the diffs answers with them; one that has
//! pruned them answers with a snapshot of the surface — reduced
//! fidelity, identical authority, since both go through the same read
//! check (v1 §4.2).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent_delegation::AgentId;
use crate::role_config::RoleRegistry;
use crate::world_caps::MAX_NAME_BYTES;

/// Audit event kind: a board was defined or edited.
pub const EVENT_BOARD_DEFINE: &str = "surface.board.define";
/// Audit event kind: an item was created.
pub const EVENT_ITEM_CREATE: &str = "surface.item.create";
/// Audit event kind: an item was claimed.
pub const EVENT_ITEM_CLAIM: &str = "surface.item.claim";
/// Audit event kind: an item moved between states.
pub const EVENT_ITEM_MOVE: &str = "surface.item.move";
/// Audit event kind: a claim was released by its holder or the owner.
pub const EVENT_ITEM_RELEASE: &str = "surface.item.release";
/// Audit event kind: a claim's lease ran out.
pub const EVENT_CLAIM_EXPIRE: &str = "surface.item.claim_expire";
/// Audit event kind: a document was defined or edited as configuration.
pub const EVENT_DOCUMENT_DEFINE: &str = "surface.document.define";
/// Audit event kind: a document's content was edited.
pub const EVENT_DOCUMENT_EDIT: &str = "surface.document.edit";
/// Audit event kind: a document's review state changed.
pub const EVENT_DOCUMENT_REVIEW: &str = "surface.document.review";
/// Audit event kind: a surface was read (recorded only when refused).
pub const EVENT_SURFACE_READ: &str = "surface.read";

/// The longest item title, in bytes.
pub const MAX_TITLE_BYTES: usize = 200;
/// The longest item description, in bytes.
pub const MAX_DESCRIPTION_BYTES: usize = 8 * 1024;
/// The longest document, in bytes.
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
/// How many changes a host keeps for catch-up by default.
pub const DEFAULT_RETAINED_CHANGES: usize = 1024;

/// Who acts on a surface.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceActor {
    /// The world's owner, who may act on every surface.
    Owner {
        /// Owner key hex or label, as supplied.
        label: String,
    },
    /// An agent acting under one of its roles.
    Agent {
        /// The agent key, lower-case hex.
        agent: String,
        /// The role it acts under.
        role: String,
    },
    /// The host itself, for lease expiry.
    Host,
}

impl SurfaceActor {
    /// An agent acting under `role`.
    #[must_use]
    pub fn agent(agent: AgentId, role: &str) -> Self {
        Self::Agent {
            agent: agent.to_hex(),
            role: role.to_owned(),
        }
    }

    /// The owner, by key hex or label.
    #[must_use]
    pub fn owner(label: &str) -> Self {
        Self::Owner {
            label: label.to_owned(),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Owner { label } => format!("owner:{label}"),
            Self::Agent { agent, role } => format!("agent:{agent}/{role}"),
            Self::Host => "host".to_owned(),
        }
    }
}

/// What a role may do on a board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct BoardPermissions {
    /// See the board and its items.
    #[serde(default)]
    pub read: bool,
    /// Create items.
    #[serde(default)]
    pub create: bool,
    /// Claim and release items.
    #[serde(default)]
    pub claim: bool,
    /// Move items between open states.
    #[serde(default)]
    pub move_items: bool,
    /// Move items into a closing state.
    #[serde(default)]
    pub close: bool,
}

/// A board's configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardDefinition {
    /// The board's name, unique in the world.
    pub name: String,
    /// Its states in column order; new items start in the first.
    pub states: Vec<String>,
    /// The states that close an item; a subset of `states`, not the
    /// first.
    #[serde(default)]
    pub closing_states: Vec<String>,
    /// Role name to what that role may do here. A role not listed may
    /// do nothing, read included.
    #[serde(default)]
    pub roles: BTreeMap<String, BoardPermissions>,
    /// How long a claim lasts, seconds.
    pub claim_lease_secs: u64,
}

impl BoardDefinition {
    /// A starter board any world may copy and edit: todo, doing,
    /// review, done; a "participant" may create, claim and move; an
    /// "observer" may read; closing is the owner's.
    #[must_use]
    pub fn starter(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            states: ["todo", "doing", "review", "done"]
                .map(str::to_owned)
                .to_vec(),
            closing_states: vec!["done".to_owned()],
            roles: BTreeMap::from([
                (
                    "participant".to_owned(),
                    BoardPermissions {
                        read: true,
                        create: true,
                        claim: true,
                        move_items: true,
                        close: false,
                    },
                ),
                (
                    "observer".to_owned(),
                    BoardPermissions {
                        read: true,
                        ..BoardPermissions::default()
                    },
                ),
            ]),
            claim_lease_secs: 60 * 60,
        }
    }

    fn validate(&self) -> Result<(), SurfaceError> {
        check_name(&self.name)?;
        let mut seen = BTreeSet::new();
        for state in &self.states {
            check_name(state)?;
            if !seen.insert(state.as_str()) {
                return Err(SurfaceError::BadDefinition);
            }
        }
        let Some(first) = self.states.first() else {
            return Err(SurfaceError::BadDefinition);
        };
        if self
            .closing_states
            .iter()
            .any(|s| !seen.contains(s.as_str()) || s == first)
        {
            return Err(SurfaceError::BadDefinition);
        }
        for role in self.roles.keys() {
            check_name(role)?;
        }
        if self.claim_lease_secs == 0 {
            return Err(SurfaceError::BadDefinition);
        }
        Ok(())
    }

    fn permissions(&self, role: &str) -> BoardPermissions {
        self.roles.get(role).copied().unwrap_or_default()
    }

    fn is_closing(&self, state: &str) -> bool {
        self.closing_states.iter().any(|s| s == state)
    }
}

/// An item's priority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Low.
    Low,
    /// Normal.
    #[default]
    Normal,
    /// High.
    High,
    /// Urgent.
    Urgent,
}

/// What a creator supplies for a new item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDraft {
    /// The title.
    pub title: String,
    /// The description.
    #[serde(default)]
    pub description: String,
    /// The priority.
    #[serde(default)]
    pub priority: Priority,
    /// Items on the same board this one waits on.
    #[serde(default)]
    pub blocked_by: BTreeSet<u64>,
    /// References to artifacts the item produced or needs.
    #[serde(default)]
    pub artifacts: Vec<String>,
}

/// A live claim on an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// The agent and role holding it.
    pub holder: SurfaceActor,
    /// The item's claim epoch: 1 for its first claim, +1 per claim.
    pub epoch: u64,
    /// When the lease runs out, unix seconds.
    pub expires_at: u64,
    /// The item's state when claimed; restored if the lease runs out.
    pub state_at_claim: String,
}

/// One item on a board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskItem {
    /// The item's id, unique on its board.
    pub id: u64,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// The current state.
    pub state: String,
    /// Who created it.
    pub creator: SurfaceActor,
    /// The priority.
    pub priority: Priority,
    /// Items on the same board this one waits on.
    pub blocked_by: BTreeSet<u64>,
    /// Artifact references.
    pub artifacts: Vec<String>,
    /// The live claim, if any.
    pub claim: Option<Claim>,
    /// How many times it has been claimed.
    pub claim_epoch: u64,
    /// Bumped by every change; claims and moves must name it.
    pub version: u64,
}

/// A board and its items, as a reader sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardView {
    /// The configuration in force.
    pub definition: BoardDefinition,
    /// Its version: 1 when defined, +1 per edit.
    pub version: u64,
    /// The items, by id.
    pub items: Vec<TaskItem>,
}

/// How concurrent edits to a document are resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictModel {
    /// Edits only append; nothing written is ever replaced.
    AppendOnly,
    /// The last edit wins; the replaced text stays in history.
    LastWriterWins,
}

/// What a role may do on a document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentPermissions {
    /// See the document and its history.
    #[serde(default)]
    pub read: bool,
    /// Edit its content.
    #[serde(default)]
    pub edit: bool,
    /// Change its review state.
    #[serde(default)]
    pub review: bool,
}

/// A document's configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentDefinition {
    /// The document's name, unique in the world.
    pub name: String,
    /// Its conflict model.
    pub conflict_model: ConflictModel,
    /// Role name to what that role may do here.
    #[serde(default)]
    pub roles: BTreeMap<String, DocumentPermissions>,
}

impl DocumentDefinition {
    fn validate(&self) -> Result<(), SurfaceError> {
        check_name(&self.name)?;
        for role in self.roles.keys() {
            check_name(role)?;
        }
        Ok(())
    }

    fn permissions(&self, role: &str) -> DocumentPermissions {
        self.roles.get(role).copied().unwrap_or_default()
    }
}

/// A document's review state. Any edit returns it to draft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Being written.
    #[default]
    Draft,
    /// Ready for review.
    InReview,
    /// A reviewer asked for changes.
    ChangesRequested,
    /// A reviewer approved this revision.
    Approved,
}

/// One edit to a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Add text at the end.
    Append {
        /// The text.
        text: String,
    },
    /// Replace the whole content (last-writer-wins documents only).
    Replace {
        /// The new content.
        text: String,
    },
}

impl EditOp {
    fn text(&self) -> &str {
        match self {
            Self::Append { text } | Self::Replace { text } => text,
        }
    }
}

/// One attributed edit in a document's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentEdit {
    /// The revision this edit produced.
    pub revision: u64,
    /// Who made it, under which role.
    pub author: SurfaceActor,
    /// The edit.
    pub op: EditOp,
    /// SHA-256 of the edit's text, lower-case hex.
    pub payload_hash: String,
    /// For a replace, the content it replaced; kept so no overwrite is
    /// unrecorded.
    pub replaced: Option<String>,
    /// When, unix seconds.
    pub at: u64,
}

/// A document as a reader sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentView {
    /// The configuration in force.
    pub definition: DocumentDefinition,
    /// Its configuration version.
    pub version: u64,
    /// The current content.
    pub content: String,
    /// The content revision: 0 before the first edit.
    pub revision: u64,
    /// The review state.
    pub review: ReviewState,
    /// Every edit, oldest first.
    pub history: Vec<DocumentEdit>,
}

/// Names one surface.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum SurfaceId {
    /// A board.
    Board(String),
    /// A document.
    Document(String),
}

/// What changed, semantically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum SemanticDiff {
    /// The surface's configuration was defined or edited.
    Configured {
        /// The new configuration version.
        version: u64,
    },
    /// An item was created.
    ItemCreated {
        /// The item.
        item: u64,
    },
    /// An item was claimed.
    ItemClaimed {
        /// The item.
        item: u64,
        /// The holder.
        holder: SurfaceActor,
        /// The claim epoch.
        epoch: u64,
    },
    /// An item moved.
    ItemMoved {
        /// The item.
        item: u64,
        /// From this state.
        from: String,
        /// To this state.
        to: String,
    },
    /// A claim was released.
    ItemReleased {
        /// The item.
        item: u64,
    },
    /// A claim's lease ran out and the item returned to its prior state.
    ClaimExpired {
        /// The item.
        item: u64,
        /// The state restored.
        restored: String,
    },
    /// A document's content changed.
    DocumentEdited {
        /// The new revision.
        revision: u64,
        /// The edit's payload hash.
        payload_hash: String,
    },
    /// A document's review state changed.
    ReviewChanged {
        /// The new state.
        review: ReviewState,
    },
}

/// One accepted change, under the world-wide surface revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceChange {
    /// The world-wide surface revision, from 1.
    pub rev: u64,
    /// The surface changed.
    pub surface: SurfaceId,
    /// What changed.
    pub diff: SemanticDiff,
    /// Who changed it.
    pub actor: SurfaceActor,
    /// When, unix seconds.
    pub at: u64,
}

/// A surface's current state, for a catch-up the diffs no longer cover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceView {
    /// A board.
    Board(BoardView),
    /// A document.
    Document(DocumentView),
}

/// The answer to a subscriber catching up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatchUp {
    /// Every change to the surface after the subscriber's revision, in
    /// order (possibly none).
    Diffs {
        /// The world-wide revision the subscriber is now current to.
        through: u64,
        /// The changes.
        changes: Vec<SurfaceChange>,
    },
    /// The host no longer holds every diff since the subscriber's
    /// revision, so it sends the surface whole: reduced fidelity,
    /// identical authority.
    Snapshot {
        /// The world-wide revision the snapshot is current to.
        through: u64,
        /// The surface.
        view: SurfaceView,
    },
}

/// Why a surface act was refused. Each maps to a stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceError {
    /// A configuration is malformed.
    BadDefinition,
    /// An input (title, description, document size) is empty or out of
    /// bounds.
    BadInput,
    /// No such board or document.
    UnknownSurface,
    /// No such item on the board.
    UnknownItem,
    /// The agent does not hold the role it acts under.
    UngrantedRole,
    /// The role may not do this on this surface.
    NotPermitted,
    /// The item changed since the version the actor saw.
    Stale {
        /// The version the actor named.
        seen: u64,
        /// The item's version now.
        current: u64,
    },
    /// Another live claim holds the item.
    Contended {
        /// The holder.
        holder: SurfaceActor,
    },
    /// The item is closed and cannot be claimed.
    Closed,
    /// The actor does not hold the item's claim.
    NotHolder,
    /// No such state on the board, or the item is already in it.
    BadState,
    /// The edit does not fit the document's conflict model.
    WrongModel,
}

impl SurfaceError {
    /// Stable machine-readable code, as journaled in an event outcome.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadDefinition => "bad_definition",
            Self::BadInput => "bad_input",
            Self::UnknownSurface => "unknown_surface",
            Self::UnknownItem => "unknown_item",
            Self::UngrantedRole => "ungranted_role",
            Self::NotPermitted => "not_permitted",
            Self::Stale { .. } => "stale",
            Self::Contended { .. } => "contended",
            Self::Closed => "closed",
            Self::NotHolder => "not_holder",
            Self::BadState => "bad_state",
            Self::WrongModel => "wrong_model",
        }
    }
}

/// One audited act on a surface: every mutation, and every refusal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceEvent {
    /// One of the `EVENT_*` kinds.
    pub kind: String,
    /// The world.
    pub world: String,
    /// The surface acted on.
    pub surface: SurfaceId,
    /// The item, when the act names one.
    pub item: Option<u64>,
    /// Who acted, as `owner:…`, `agent:<key>/<role>` or `host`.
    pub actor: String,
    /// `executed`, or `refused:<code>`.
    pub outcome: String,
    /// When, unix seconds.
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Board {
    definition: BoardDefinition,
    version: u64,
    items: BTreeMap<u64, TaskItem>,
    next_item: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Document {
    definition: DocumentDefinition,
    version: u64,
    content: String,
    review: ReviewState,
    history: Vec<DocumentEdit>,
}

impl Board {
    fn view(&self) -> BoardView {
        BoardView {
            definition: self.definition.clone(),
            version: self.version,
            items: self.items.values().cloned().collect(),
        }
    }
}

impl Document {
    fn view(&self) -> DocumentView {
        DocumentView {
            definition: self.definition.clone(),
            version: self.version,
            content: self.content.clone(),
            revision: self.history.last().map_or(0, |e| e.revision),
            review: self.review,
            history: self.history.clone(),
        }
    }
}

/// One world's boards and documents, their change log and audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkSurfaces {
    world: String,
    boards: BTreeMap<String, Board>,
    documents: BTreeMap<String, Document>,
    changes: VecDeque<SurfaceChange>,
    rev: u64,
    pruned_through: u64,
    retained: usize,
    events: Vec<SurfaceEvent>,
}

enum Perm {
    Read,
    Create,
    Claim,
    Move,
    Close,
    Edit,
    Review,
}

impl WorkSurfaces {
    /// No surfaces yet, keeping [`DEFAULT_RETAINED_CHANGES`] for catch-up.
    #[must_use]
    pub fn new(world: impl Into<String>) -> Self {
        Self::with_retention(world, DEFAULT_RETAINED_CHANGES)
    }

    /// No surfaces yet, keeping at most `retained` changes for catch-up.
    #[must_use]
    pub fn with_retention(world: impl Into<String>, retained: usize) -> Self {
        Self {
            world: world.into(),
            boards: BTreeMap::new(),
            documents: BTreeMap::new(),
            changes: VecDeque::new(),
            rev: 0,
            pruned_through: 0,
            retained,
            events: Vec::new(),
        }
    }

    /// The world these surfaces belong to.
    #[must_use]
    pub fn world(&self) -> &str {
        &self.world
    }

    /// The world-wide surface revision: how many changes were accepted.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.rev
    }

    /// Every audit event, oldest first, refusals included.
    #[must_use]
    pub fn events(&self) -> &[SurfaceEvent] {
        &self.events
    }

    /// Define or edit a board. Only the owner configures surfaces.
    ///
    /// # Errors
    ///
    /// The actor is not the owner, or the definition is malformed.
    pub fn define_board(
        &mut self,
        actor: &SurfaceActor,
        definition: BoardDefinition,
        now: u64,
    ) -> Result<u64, SurfaceError> {
        let surface = SurfaceId::Board(definition.name.clone());
        let result = (|| {
            owner_only(actor)?;
            definition.validate()?;
            Ok(())
        })();
        let result = result.map(|()| {
            let board = self
                .boards
                .entry(definition.name.clone())
                .or_insert_with(|| Board {
                    definition: definition.clone(),
                    version: 0,
                    items: BTreeMap::new(),
                    next_item: 1,
                });
            board.definition = definition;
            board.version += 1;
            board.version
        });
        if let Ok(version) = result {
            self.change(&surface, SemanticDiff::Configured { version }, actor, now);
        }
        self.audit(EVENT_BOARD_DEFINE, &surface, None, actor, &result, now);
        result
    }

    /// Define or edit a document's configuration. Only the owner
    /// configures surfaces; content is kept across edits.
    ///
    /// # Errors
    ///
    /// The actor is not the owner, the definition is malformed, or it
    /// would turn a document with replaced text into an append-only one
    /// (append-only must hold over the whole history).
    pub fn define_document(
        &mut self,
        actor: &SurfaceActor,
        definition: DocumentDefinition,
        now: u64,
    ) -> Result<u64, SurfaceError> {
        let surface = SurfaceId::Document(definition.name.clone());
        let existing = self.documents.get(&definition.name);
        let result = (|| {
            owner_only(actor)?;
            definition.validate()?;
            if definition.conflict_model == ConflictModel::AppendOnly
                && existing.is_some_and(|d| d.history.iter().any(|e| e.replaced.is_some()))
            {
                return Err(SurfaceError::WrongModel);
            }
            Ok(())
        })();
        let result = result.map(|()| {
            let document = self
                .documents
                .entry(definition.name.clone())
                .or_insert_with(|| Document {
                    definition: definition.clone(),
                    version: 0,
                    content: String::new(),
                    review: ReviewState::Draft,
                    history: Vec::new(),
                });
            document.definition = definition;
            document.version += 1;
            document.version
        });
        if let Ok(version) = result {
            self.change(&surface, SemanticDiff::Configured { version }, actor, now);
        }
        self.audit(EVENT_DOCUMENT_DEFINE, &surface, None, actor, &result, now);
        result
    }

    /// Read a board.
    ///
    /// # Errors
    ///
    /// No such board, or the actor may not read it (audited).
    pub fn board(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        name: &str,
        now: u64,
    ) -> Result<BoardView, SurfaceError> {
        let surface = SurfaceId::Board(name.to_owned());
        let result = self
            .read_view(registry, actor, &surface)
            .map(|view| match view {
                SurfaceView::Board(board) => board,
                SurfaceView::Document(_) => unreachable_view(),
            });
        if result.is_err() {
            self.audit(EVENT_SURFACE_READ, &surface, None, actor, &result, now);
        }
        result
    }

    /// Read a document.
    ///
    /// # Errors
    ///
    /// No such document, or the actor may not read it (audited).
    pub fn document(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        name: &str,
        now: u64,
    ) -> Result<DocumentView, SurfaceError> {
        let surface = SurfaceId::Document(name.to_owned());
        let result = self
            .read_view(registry, actor, &surface)
            .map(|view| match view {
                SurfaceView::Document(document) => document,
                SurfaceView::Board(_) => unreachable_document(),
            });
        if result.is_err() {
            self.audit(EVENT_SURFACE_READ, &surface, None, actor, &result, now);
        }
        result
    }

    /// Rebuild a document's content as of `revision` from its history.
    ///
    /// # Errors
    ///
    /// No such document or revision, or the actor may not read it.
    pub fn document_at(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        name: &str,
        revision: u64,
        now: u64,
    ) -> Result<String, SurfaceError> {
        let view = self.document(registry, actor, name, now)?;
        if revision > view.revision {
            return Err(SurfaceError::BadInput);
        }
        let mut content = String::new();
        for edit in view.history.iter().take_while(|e| e.revision <= revision) {
            match &edit.op {
                EditOp::Append { text } => content.push_str(text),
                EditOp::Replace { text } => content.clone_from(text),
            }
        }
        Ok(content)
    }

    /// Create an item in the board's first state.
    ///
    /// # Errors
    ///
    /// No such board, the actor may not create here, or the draft is
    /// out of bounds or names an unknown blocking item.
    pub fn create_item(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        board: &str,
        draft: ItemDraft,
        now: u64,
    ) -> Result<u64, SurfaceError> {
        let surface = SurfaceId::Board(board.to_owned());
        let result = self
            .authorize(registry, actor, &surface, &Perm::Create)
            .and_then(|()| {
                check_text(&draft.title, MAX_TITLE_BYTES, false)?;
                check_text(&draft.description, MAX_DESCRIPTION_BYTES, true)?;
                let board = self.board_mut(board)?;
                if draft
                    .blocked_by
                    .iter()
                    .any(|id| !board.items.contains_key(id))
                {
                    return Err(SurfaceError::UnknownItem);
                }
                let id = board.next_item;
                board.next_item += 1;
                let state = board.definition.states.first().cloned().unwrap_or_default();
                board.items.insert(
                    id,
                    TaskItem {
                        id,
                        title: draft.title,
                        description: draft.description,
                        state,
                        creator: actor.clone(),
                        priority: draft.priority,
                        blocked_by: draft.blocked_by,
                        artifacts: draft.artifacts,
                        claim: None,
                        claim_epoch: 0,
                        version: 1,
                    },
                );
                Ok(id)
            });
        if let Ok(item) = result {
            self.change(&surface, SemanticDiff::ItemCreated { item }, actor, now);
        }
        let item = result.as_ref().ok().copied();
        self.audit(EVENT_ITEM_CREATE, &surface, item, actor, &result, now);
        result
    }

    /// Claim an item, naming the version the claimer saw. The claim
    /// lasts the board's lease.
    ///
    /// # Errors
    ///
    /// No such board or item, the actor may not claim, the item moved
    /// since `seen_version` (stale), another live claim holds it
    /// (contended), or it is closed. Every refusal is audited.
    pub fn claim(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        board: &str,
        item: u64,
        seen_version: u64,
        now: u64,
    ) -> Result<Claim, SurfaceError> {
        let surface = SurfaceId::Board(board.to_owned());
        let result = self
            .authorize(registry, actor, &surface, &Perm::Claim)
            .and_then(|()| {
                let board = self.board_mut(board)?;
                let lease = board.definition.claim_lease_secs;
                let closing = board.definition.closing_states.clone();
                let entry = board
                    .items
                    .get_mut(&item)
                    .ok_or(SurfaceError::UnknownItem)?;
                if entry.version != seen_version {
                    return Err(SurfaceError::Stale {
                        seen: seen_version,
                        current: entry.version,
                    });
                }
                if let Some(claim) = entry.claim.as_ref().filter(|c| c.expires_at > now) {
                    return Err(SurfaceError::Contended {
                        holder: claim.holder.clone(),
                    });
                }
                if closing.contains(&entry.state) {
                    return Err(SurfaceError::Closed);
                }
                entry.claim_epoch += 1;
                let claim = Claim {
                    holder: actor.clone(),
                    epoch: entry.claim_epoch,
                    expires_at: now.saturating_add(lease),
                    state_at_claim: entry.state.clone(),
                };
                entry.claim = Some(claim.clone());
                entry.version += 1;
                Ok(claim)
            });
        if let Ok(claim) = &result {
            self.change(
                &surface,
                SemanticDiff::ItemClaimed {
                    item,
                    holder: claim.holder.clone(),
                    epoch: claim.epoch,
                },
                actor,
                now,
            );
        }
        self.audit(EVENT_ITEM_CLAIM, &surface, Some(item), actor, &result, now);
        result
    }

    /// Move an item to another state, naming the version the actor saw.
    /// Moving into a closing state needs the close permission, and ends
    /// any claim. While another agent's live claim holds the item, only
    /// that holder (or the owner) may move it.
    ///
    /// # Errors
    ///
    /// No such board, item or state, the actor may not move or close,
    /// the item is stale, or another agent holds it.
    pub fn move_item(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        board: &str,
        item: u64,
        to: &str,
        seen_version: u64,
        now: u64,
    ) -> Result<u64, SurfaceError> {
        let surface = SurfaceId::Board(board.to_owned());
        let closing = self
            .boards
            .get(board)
            .is_some_and(|b| b.definition.is_closing(to));
        let perm = if closing { Perm::Close } else { Perm::Move };
        let mut from = String::new();
        let result = self
            .authorize(registry, actor, &surface, &perm)
            .and_then(|()| {
                let board = self.board_mut(board)?;
                if !board.definition.states.iter().any(|s| s == to) {
                    return Err(SurfaceError::BadState);
                }
                let entry = board
                    .items
                    .get_mut(&item)
                    .ok_or(SurfaceError::UnknownItem)?;
                if entry.version != seen_version {
                    return Err(SurfaceError::Stale {
                        seen: seen_version,
                        current: entry.version,
                    });
                }
                if entry.state == to {
                    return Err(SurfaceError::BadState);
                }
                if let Some(claim) = entry.claim.as_ref().filter(|c| c.expires_at > now)
                    && claim.holder != *actor
                    && !matches!(actor, SurfaceActor::Owner { .. })
                {
                    return Err(SurfaceError::Contended {
                        holder: claim.holder.clone(),
                    });
                }
                from = std::mem::replace(&mut entry.state, to.to_owned());
                if closing {
                    entry.claim = None;
                }
                entry.version += 1;
                Ok(entry.version)
            });
        if result.is_ok() {
            self.change(
                &surface,
                SemanticDiff::ItemMoved {
                    item,
                    from,
                    to: to.to_owned(),
                },
                actor,
                now,
            );
        }
        self.audit(EVENT_ITEM_MOVE, &surface, Some(item), actor, &result, now);
        result
    }

    /// Release a claim: its holder, or the owner, ends it; the item keeps
    /// its state.
    ///
    /// # Errors
    ///
    /// No such board or item, or the actor does not hold a live claim
    /// on it (and is not the owner).
    pub fn release(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        board: &str,
        item: u64,
        now: u64,
    ) -> Result<(), SurfaceError> {
        let surface = SurfaceId::Board(board.to_owned());
        let result = self
            .authorize(registry, actor, &surface, &Perm::Claim)
            .and_then(|()| {
                let board = self.board_mut(board)?;
                let entry = board
                    .items
                    .get_mut(&item)
                    .ok_or(SurfaceError::UnknownItem)?;
                let holds = entry.claim.as_ref().is_some_and(|c| {
                    c.expires_at > now
                        && (c.holder == *actor || matches!(actor, SurfaceActor::Owner { .. }))
                });
                if !holds {
                    return Err(SurfaceError::NotHolder);
                }
                entry.claim = None;
                entry.version += 1;
                Ok(())
            });
        if result.is_ok() {
            self.change(&surface, SemanticDiff::ItemReleased { item }, actor, now);
        }
        self.audit(
            EVENT_ITEM_RELEASE,
            &surface,
            Some(item),
            actor,
            &result,
            now,
        );
        result
    }

    /// Expire every claim whose lease ran out by `now`: each item returns
    /// to the state it had when claimed, as an audited host event.
    /// Returns the (board, item) pairs expired.
    pub fn expire_claims(&mut self, now: u64) -> Vec<(String, u64)> {
        let mut expired = Vec::new();
        for (name, board) in &mut self.boards {
            for entry in board.items.values_mut() {
                let Some(claim) = entry.claim.take_if(|c| c.expires_at <= now) else {
                    continue;
                };
                entry.state = claim.state_at_claim;
                entry.version += 1;
                expired.push((name.clone(), entry.id, entry.state.clone()));
            }
        }
        let mut done = Vec::new();
        for (board, item, restored) in expired {
            let surface = SurfaceId::Board(board.clone());
            self.change(
                &surface,
                SemanticDiff::ClaimExpired { item, restored },
                &SurfaceActor::Host,
                now,
            );
            self.audit(
                EVENT_CLAIM_EXPIRE,
                &surface,
                Some(item),
                &SurfaceActor::Host,
                &Ok::<(), SurfaceError>(()),
                now,
            );
            done.push((board, item));
        }
        done
    }

    /// Edit a document. An append-only document takes only appends; a
    /// last-writer-wins document takes appends and replaces, keeping
    /// the replaced text in history. Any edit returns the review state
    /// to draft.
    ///
    /// # Errors
    ///
    /// No such document, the actor may not edit, the edit does not fit
    /// the conflict model, or the result is out of bounds.
    pub fn edit_document(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        name: &str,
        op: EditOp,
        now: u64,
    ) -> Result<u64, SurfaceError> {
        let surface = SurfaceId::Document(name.to_owned());
        let result = self
            .authorize(registry, actor, &surface, &Perm::Edit)
            .and_then(|()| {
                let document = self
                    .documents
                    .get_mut(name)
                    .ok_or(SurfaceError::UnknownSurface)?;
                if op.text().is_empty() {
                    return Err(SurfaceError::BadInput);
                }
                let (next, replaced) = match &op {
                    EditOp::Append { text } => (format!("{}{text}", document.content), None),
                    EditOp::Replace { text } => {
                        if document.definition.conflict_model == ConflictModel::AppendOnly {
                            return Err(SurfaceError::WrongModel);
                        }
                        (text.clone(), Some(document.content.clone()))
                    }
                };
                if next.len() > MAX_DOCUMENT_BYTES {
                    return Err(SurfaceError::BadInput);
                }
                let revision = document.history.last().map_or(1, |e| e.revision + 1);
                let payload_hash = sha256_hex(op.text().as_bytes());
                document.history.push(DocumentEdit {
                    revision,
                    author: actor.clone(),
                    op,
                    payload_hash: payload_hash.clone(),
                    replaced,
                    at: now,
                });
                document.content = next;
                document.review = ReviewState::Draft;
                Ok((revision, payload_hash))
            });
        if let Ok((revision, payload_hash)) = &result {
            self.change(
                &surface,
                SemanticDiff::DocumentEdited {
                    revision: *revision,
                    payload_hash: payload_hash.clone(),
                },
                actor,
                now,
            );
        }
        let result = result.map(|(revision, _)| revision);
        self.audit(EVENT_DOCUMENT_EDIT, &surface, None, actor, &result, now);
        result
    }

    /// Set a document's review state.
    ///
    /// # Errors
    ///
    /// No such document, or the actor may not review it.
    pub fn review_document(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        name: &str,
        review: ReviewState,
        now: u64,
    ) -> Result<(), SurfaceError> {
        let surface = SurfaceId::Document(name.to_owned());
        let result = self
            .authorize(registry, actor, &surface, &Perm::Review)
            .and_then(|()| {
                let document = self
                    .documents
                    .get_mut(name)
                    .ok_or(SurfaceError::UnknownSurface)?;
                document.review = review;
                Ok(())
            });
        if result.is_ok() {
            self.change(&surface, SemanticDiff::ReviewChanged { review }, actor, now);
        }
        self.audit(EVENT_DOCUMENT_REVIEW, &surface, None, actor, &result, now);
        result
    }

    /// Everything that changed on `surface` after world-wide revision
    /// `since`, under the same read check as a direct read: the diffs
    /// when this host still holds them all, otherwise a snapshot.
    ///
    /// # Errors
    ///
    /// No such surface, or the actor may not read it (audited).
    pub fn catch_up(
        &mut self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        surface: &SurfaceId,
        since: u64,
        now: u64,
    ) -> Result<CatchUp, SurfaceError> {
        let view = match self.read_view(registry, actor, surface) {
            Ok(view) => view,
            Err(error) => {
                let result = Err::<(), _>(error.clone());
                self.audit(EVENT_SURFACE_READ, surface, None, actor, &result, now);
                return Err(error);
            }
        };
        if since >= self.pruned_through {
            let changes = self
                .changes
                .iter()
                .filter(|c| c.rev > since && c.surface == *surface)
                .cloned()
                .collect();
            Ok(CatchUp::Diffs {
                through: self.rev,
                changes,
            })
        } else {
            Ok(CatchUp::Snapshot {
                through: self.rev,
                view,
            })
        }
    }

    fn read_view(
        &self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        surface: &SurfaceId,
    ) -> Result<SurfaceView, SurfaceError> {
        self.authorize(registry, actor, surface, &Perm::Read)?;
        match surface {
            SurfaceId::Board(name) => self
                .boards
                .get(name)
                .map(|b| SurfaceView::Board(b.view()))
                .ok_or(SurfaceError::UnknownSurface),
            SurfaceId::Document(name) => self
                .documents
                .get(name)
                .map(|d| SurfaceView::Document(d.view()))
                .ok_or(SurfaceError::UnknownSurface),
        }
    }

    fn authorize(
        &self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        surface: &SurfaceId,
        perm: &Perm,
    ) -> Result<(), SurfaceError> {
        let exists = match surface {
            SurfaceId::Board(name) => self.boards.contains_key(name),
            SurfaceId::Document(name) => self.documents.contains_key(name),
        };
        if !exists {
            return Err(SurfaceError::UnknownSurface);
        }
        let (agent, role) = match actor {
            SurfaceActor::Owner { .. } => return Ok(()),
            SurfaceActor::Host => return Err(SurfaceError::NotPermitted),
            SurfaceActor::Agent { agent, role } => (agent, role),
        };
        if registry.world() != self.world
            || !registry
                .list_grants()
                .iter()
                .any(|g| g.role == *role && g.agent.to_hex() == *agent)
        {
            return Err(SurfaceError::UngrantedRole);
        }
        let allowed = match surface {
            SurfaceId::Board(name) => {
                let p = self
                    .boards
                    .get(name)
                    .map(|b| b.definition.permissions(role))
                    .unwrap_or_default();
                match perm {
                    Perm::Read => p.read,
                    Perm::Create => p.create,
                    Perm::Claim => p.claim,
                    Perm::Move => p.move_items,
                    Perm::Close => p.close,
                    Perm::Edit | Perm::Review => false,
                }
            }
            SurfaceId::Document(name) => {
                let p = self
                    .documents
                    .get(name)
                    .map(|d| d.definition.permissions(role))
                    .unwrap_or_default();
                match perm {
                    Perm::Read => p.read,
                    Perm::Edit => p.edit,
                    Perm::Review => p.review,
                    Perm::Create | Perm::Claim | Perm::Move | Perm::Close => false,
                }
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(SurfaceError::NotPermitted)
        }
    }

    fn board_mut(&mut self, name: &str) -> Result<&mut Board, SurfaceError> {
        self.boards
            .get_mut(name)
            .ok_or(SurfaceError::UnknownSurface)
    }

    fn change(&mut self, surface: &SurfaceId, diff: SemanticDiff, actor: &SurfaceActor, at: u64) {
        self.rev += 1;
        self.changes.push_back(SurfaceChange {
            rev: self.rev,
            surface: surface.clone(),
            diff,
            actor: actor.clone(),
            at,
        });
        while self.changes.len() > self.retained {
            if let Some(dropped) = self.changes.pop_front() {
                self.pruned_through = dropped.rev;
            }
        }
    }

    fn audit<T>(
        &mut self,
        kind: &str,
        surface: &SurfaceId,
        item: Option<u64>,
        actor: &SurfaceActor,
        result: &Result<T, SurfaceError>,
        at: u64,
    ) {
        let outcome = match result {
            Ok(_) => "executed".to_owned(),
            Err(error) => format!("refused:{}", error.code()),
        };
        self.events.push(SurfaceEvent {
            kind: kind.to_owned(),
            world: self.world.clone(),
            surface: surface.clone(),
            item,
            actor: actor.label(),
            outcome,
            at,
        });
    }
}

const fn owner_only(actor: &SurfaceActor) -> Result<(), SurfaceError> {
    if matches!(actor, SurfaceActor::Owner { .. }) {
        Ok(())
    } else {
        Err(SurfaceError::NotPermitted)
    }
}

fn check_name(name: &str) -> Result<(), SurfaceError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(SurfaceError::BadDefinition);
    }
    Ok(())
}

fn check_text(text: &str, max: usize, may_be_empty: bool) -> Result<(), SurfaceError> {
    if (!may_be_empty && text.trim().is_empty()) || text.len() > max {
        return Err(SurfaceError::BadInput);
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `read_view` on a board id always yields a board view.
fn unreachable_view() -> BoardView {
    BoardView {
        definition: BoardDefinition::starter("unreachable"),
        version: 0,
        items: Vec::new(),
    }
}

/// `read_view` on a document id always yields a document view.
fn unreachable_document() -> DocumentView {
    DocumentView {
        definition: DocumentDefinition {
            name: "unreachable".to_owned(),
            conflict_model: ConflictModel::AppendOnly,
            roles: BTreeMap::new(),
        },
        version: 0,
        content: String::new(),
        revision: 0,
        review: ReviewState::Draft,
        history: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const WORLD: &str = "studio";
    const T0: u64 = 1_800_000_000;

    fn agent(byte: u8) -> AgentId {
        AgentId([byte; 32])
    }

    /// Roles: participant (worker), observer (visitor), and a
    /// "reviewer" copied from participant. Agents: 1 and 2 are
    /// participants, 3 is an observer, 4 a reviewer.
    fn registry() -> RoleRegistry {
        let mut registry = RoleRegistry::new(WORLD);
        registry.install_starters("owner", T0);
        let mut reviewer = registry.record("participant").unwrap().definition.clone();
        reviewer.name = "reviewer".to_owned();
        assert!(registry.upsert(reviewer, "owner", T0).decision.is_ok());
        for (byte, role) in [
            (1, "participant"),
            (2, "participant"),
            (3, "observer"),
            (4, "reviewer"),
        ] {
            assert!(
                registry
                    .grant(role, agent(byte), "owner", T0)
                    .decision
                    .is_ok()
            );
        }
        registry
    }

    fn worker(byte: u8) -> SurfaceActor {
        SurfaceActor::agent(agent(byte), "participant")
    }

    fn visitor() -> SurfaceActor {
        SurfaceActor::agent(agent(3), "observer")
    }

    fn reviewer() -> SurfaceActor {
        SurfaceActor::agent(agent(4), "reviewer")
    }

    fn owner() -> SurfaceActor {
        SurfaceActor::owner("owner")
    }

    fn surfaces() -> WorkSurfaces {
        let mut surfaces = WorkSurfaces::new(WORLD);
        surfaces
            .define_board(&owner(), BoardDefinition::starter("work"), T0)
            .unwrap();
        surfaces
            .define_document(
                &owner(),
                DocumentDefinition {
                    name: "notes".to_owned(),
                    conflict_model: ConflictModel::LastWriterWins,
                    roles: BTreeMap::from([
                        (
                            "participant".to_owned(),
                            DocumentPermissions {
                                read: true,
                                edit: true,
                                review: false,
                            },
                        ),
                        (
                            "reviewer".to_owned(),
                            DocumentPermissions {
                                read: true,
                                edit: false,
                                review: true,
                            },
                        ),
                    ]),
                },
                T0,
            )
            .unwrap();
        surfaces
    }

    fn draft(title: &str) -> ItemDraft {
        ItemDraft {
            title: title.to_owned(),
            ..ItemDraft::default()
        }
    }

    fn item(s: &mut WorkSurfaces, r: &RoleRegistry, id: u64) -> TaskItem {
        s.board(r, &owner(), "work", T0)
            .unwrap()
            .items
            .into_iter()
            .find(|i| i.id == id)
            .unwrap()
    }

    fn last_outcome(s: &WorkSurfaces) -> &str {
        &s.events().last().unwrap().outcome
    }

    #[test]
    fn items_carry_the_task_schema_and_start_in_the_first_state() {
        let r = registry();
        let mut s = surfaces();
        let first = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let id = s
            .create_item(
                &r,
                &worker(1),
                "work",
                ItemDraft {
                    title: "build".to_owned(),
                    description: "the board model".to_owned(),
                    priority: Priority::High,
                    blocked_by: BTreeSet::from([first]),
                    artifacts: vec!["doc:notes".to_owned()],
                },
                T0,
            )
            .unwrap();
        let built = item(&mut s, &r, id);
        assert_eq!(built.state, "todo");
        assert_eq!(built.creator, worker(1));
        assert_eq!(built.priority, Priority::High);
        assert_eq!(built.blocked_by, BTreeSet::from([first]));
        assert_eq!(built.artifacts, vec!["doc:notes"]);
        assert_eq!(built.version, 1);
        assert!(built.claim.is_none());

        // Unknown blockers and empty titles are refused and audited.
        assert_eq!(
            s.create_item(
                &r,
                &worker(1),
                "work",
                ItemDraft {
                    title: "x".to_owned(),
                    blocked_by: BTreeSet::from([99]),
                    ..ItemDraft::default()
                },
                T0
            ),
            Err(SurfaceError::UnknownItem)
        );
        assert_eq!(last_outcome(&s), "refused:unknown_item");
        assert_eq!(
            s.create_item(&r, &worker(1), "work", draft("  "), T0),
            Err(SurfaceError::BadInput)
        );
    }

    #[test]
    fn a_claim_against_a_moved_item_fails_visibly_as_stale() {
        let r = registry();
        let mut s = surfaces();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let seen = item(&mut s, &r, id).version;

        // Agent 2 claims first.
        let claim = s.claim(&r, &worker(2), "work", id, seen, T0 + 1).unwrap();
        assert_eq!(claim.epoch, 1);
        assert_eq!(claim.holder, worker(2));

        // Agent 1 claims against the version it saw before: stale, not a
        // double assignment.
        assert_eq!(
            s.claim(&r, &worker(1), "work", id, seen, T0 + 2),
            Err(SurfaceError::Stale {
                seen,
                current: seen + 1
            })
        );
        assert_eq!(last_outcome(&s), "refused:stale");

        // With the current version it is contended, and says by whom.
        let current = item(&mut s, &r, id).version;
        assert_eq!(
            s.claim(&r, &worker(1), "work", id, current, T0 + 3),
            Err(SurfaceError::Contended { holder: worker(2) })
        );
        assert_eq!(last_outcome(&s), "refused:contended");
        assert_eq!(item(&mut s, &r, id).claim.unwrap().holder, worker(2));
    }

    #[test]
    fn an_expired_claim_returns_the_item_to_its_prior_state() {
        let r = registry();
        let mut s = surfaces();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let v = item(&mut s, &r, id).version;
        s.claim(&r, &worker(1), "work", id, v, T0).unwrap();
        let v = item(&mut s, &r, id).version;
        s.move_item(&r, &worker(1), "work", id, "doing", v, T0 + 10)
            .unwrap();

        // Before the lease ends nothing expires.
        assert!(s.expire_claims(T0 + 3599).is_empty());
        // At the lease end the item goes back to todo, unclaimed.
        assert_eq!(s.expire_claims(T0 + 3600), vec![("work".to_owned(), id)]);
        let back = item(&mut s, &r, id);
        assert_eq!(back.state, "todo");
        assert!(back.claim.is_none());
        let event = s.events().last().unwrap();
        assert_eq!(event.kind, EVENT_CLAIM_EXPIRE);
        assert_eq!(event.actor, "host");

        // Another agent can now claim it, under a new epoch.
        let claim = s
            .claim(&r, &worker(2), "work", id, back.version, T0 + 3601)
            .unwrap();
        assert_eq!(claim.epoch, 2);
    }

    #[test]
    fn moves_need_the_holder_and_closing_needs_its_own_permission() {
        let r = registry();
        let mut s = surfaces();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let v = item(&mut s, &r, id).version;
        s.claim(&r, &worker(1), "work", id, v, T0).unwrap();
        let v = item(&mut s, &r, id).version;

        // Another participant may not move an item someone else holds.
        assert_eq!(
            s.move_item(&r, &worker(2), "work", id, "doing", v, T0),
            Err(SurfaceError::Contended { holder: worker(1) })
        );
        // The holder may move it, but a participant may not close.
        let v = s
            .move_item(&r, &worker(1), "work", id, "review", v, T0)
            .unwrap();
        assert_eq!(
            s.move_item(&r, &worker(1), "work", id, "done", v, T0),
            Err(SurfaceError::NotPermitted)
        );
        assert_eq!(
            s.move_item(&r, &worker(1), "work", id, "nowhere", v, T0),
            Err(SurfaceError::BadState)
        );
        // The owner closes it, which ends the claim; a closed item cannot
        // be claimed.
        let v = s
            .move_item(&r, &owner(), "work", id, "done", v, T0)
            .unwrap();
        let closed = item(&mut s, &r, id);
        assert_eq!(closed.state, "done");
        assert!(closed.claim.is_none());
        assert_eq!(
            s.claim(&r, &worker(2), "work", id, v, T0),
            Err(SurfaceError::Closed)
        );
    }

    #[test]
    fn release_is_for_the_holder_or_owner() {
        let r = registry();
        let mut s = surfaces();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let v = item(&mut s, &r, id).version;
        s.claim(&r, &worker(1), "work", id, v, T0).unwrap();
        assert_eq!(
            s.release(&r, &worker(2), "work", id, T0),
            Err(SurfaceError::NotHolder)
        );
        s.release(&r, &worker(1), "work", id, T0).unwrap();
        assert!(item(&mut s, &r, id).claim.is_none());
        assert_eq!(
            s.release(&r, &worker(1), "work", id, T0),
            Err(SurfaceError::NotHolder)
        );
    }

    #[test]
    fn a_role_without_read_access_cannot_read_and_the_refusal_is_audited() {
        let r = registry();
        let mut s = surfaces();
        // The observer may read the starter board ...
        assert!(s.board(&r, &visitor(), "work", T0).is_ok());
        // ... but the notes document gives the observer nothing.
        assert_eq!(
            s.document(&r, &visitor(), "notes", T0),
            Err(SurfaceError::NotPermitted)
        );
        let event = s.events().last().unwrap();
        assert_eq!(event.kind, EVENT_SURFACE_READ);
        assert_eq!(event.outcome, "refused:not_permitted");
        assert_eq!(event.surface, SurfaceId::Document("notes".to_owned()));
        // Catching up is the same read, refused the same way.
        assert_eq!(
            s.catch_up(
                &r,
                &visitor(),
                &SurfaceId::Document("notes".to_owned()),
                0,
                T0
            ),
            Err(SurfaceError::NotPermitted)
        );
        // The observer may not create on the board either.
        assert_eq!(
            s.create_item(&r, &visitor(), "work", draft("x"), T0),
            Err(SurfaceError::NotPermitted)
        );
        assert_eq!(last_outcome(&s), "refused:not_permitted");
    }

    #[test]
    fn an_ungranted_role_is_refused_and_audited() {
        let r = registry();
        let mut s = surfaces();
        // Agent 3 holds observer, not participant.
        let pretender = SurfaceActor::agent(agent(3), "participant");
        assert_eq!(
            s.create_item(&r, &pretender, "work", draft("x"), T0),
            Err(SurfaceError::UngrantedRole)
        );
        assert_eq!(last_outcome(&s), "refused:ungranted_role");
        // An unknown agent likewise.
        assert_eq!(
            s.board(&r, &SurfaceActor::agent(agent(9), "observer"), "work", T0),
            Err(SurfaceError::UngrantedRole)
        );
        // A revoked grant ends the authority at once.
        let mut revoked = r.clone();
        assert!(
            revoked
                .revoke("participant", agent(1), "owner", T0)
                .decision
                .is_ok()
        );
        assert_eq!(
            s.create_item(&revoked, &worker(1), "work", draft("x"), T0),
            Err(SurfaceError::UngrantedRole)
        );
        // Only the owner configures surfaces.
        assert_eq!(
            s.define_board(&worker(1), BoardDefinition::starter("mine"), T0),
            Err(SurfaceError::NotPermitted)
        );
        assert_eq!(last_outcome(&s), "refused:not_permitted");
    }

    #[test]
    fn surface_state_survives_an_agent_leaving_and_rejoining() {
        let r = registry();
        let mut s = surfaces();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let v = item(&mut s, &r, id).version;
        s.claim(&r, &worker(1), "work", id, v, T0).unwrap();
        // The agent's session ends and a new one starts; the same key and
        // role still hold the claim and may act on it.
        let rejoined = SurfaceActor::agent(agent(1), "participant");
        let v = item(&mut s, &r, id).version;
        s.move_item(&r, &rejoined, "work", id, "doing", v, T0 + 60)
            .unwrap();
        assert_eq!(item(&mut s, &r, id).claim.unwrap().holder, worker(1));
    }

    #[test]
    fn documents_attribute_every_edit_and_keep_replaced_text() {
        let r = registry();
        let mut s = surfaces();
        assert_eq!(
            s.edit_document(
                &r,
                &worker(1),
                "notes",
                EditOp::Append {
                    text: "plan: ".to_owned()
                },
                T0
            ),
            Ok(1)
        );
        s.edit_document(
            &r,
            &worker(2),
            "notes",
            EditOp::Append {
                text: "ship it".to_owned(),
            },
            T0 + 1,
        )
        .unwrap();
        s.edit_document(
            &r,
            &worker(1),
            "notes",
            EditOp::Replace {
                text: "plan: wait".to_owned(),
            },
            T0 + 2,
        )
        .unwrap();
        let view = s.document(&r, &worker(1), "notes", T0).unwrap();
        assert_eq!(view.content, "plan: wait");
        assert_eq!(view.revision, 3);
        assert_eq!(view.history[1].author, worker(2));
        assert_eq!(view.history[1].payload_hash, sha256_hex(b"ship it"));
        assert_eq!(view.history[2].replaced.as_deref(), Some("plan: ship it"));
        // Every past revision can be rebuilt from the history.
        assert_eq!(
            s.document_at(&r, &worker(1), "notes", 2, T0).unwrap(),
            "plan: ship it"
        );
        assert_eq!(s.document_at(&r, &worker(1), "notes", 0, T0).unwrap(), "");
        // A reviewer may review but not edit.
        assert_eq!(
            s.edit_document(
                &r,
                &reviewer(),
                "notes",
                EditOp::Append {
                    text: "x".to_owned()
                },
                T0
            ),
            Err(SurfaceError::NotPermitted)
        );
        s.review_document(&r, &reviewer(), "notes", ReviewState::Approved, T0)
            .unwrap();
        assert_eq!(
            s.review_document(&r, &worker(1), "notes", ReviewState::Approved, T0),
            Err(SurfaceError::NotPermitted)
        );
        // An edit after approval returns it to draft.
        s.edit_document(
            &r,
            &worker(1),
            "notes",
            EditOp::Append {
                text: "!".to_owned(),
            },
            T0 + 3,
        )
        .unwrap();
        assert_eq!(
            s.document(&r, &worker(1), "notes", T0).unwrap().review,
            ReviewState::Draft
        );
    }

    #[test]
    fn an_append_only_document_refuses_replacement() {
        let r = registry();
        let mut s = surfaces();
        let mut log = DocumentDefinition {
            name: "log".to_owned(),
            conflict_model: ConflictModel::AppendOnly,
            roles: BTreeMap::from([(
                "participant".to_owned(),
                DocumentPermissions {
                    read: true,
                    edit: true,
                    review: false,
                },
            )]),
        };
        s.define_document(&owner(), log.clone(), T0).unwrap();
        s.edit_document(
            &r,
            &worker(1),
            "log",
            EditOp::Append {
                text: "one".to_owned(),
            },
            T0,
        )
        .unwrap();
        assert_eq!(
            s.edit_document(
                &r,
                &worker(1),
                "log",
                EditOp::Replace {
                    text: "gone".to_owned()
                },
                T0
            ),
            Err(SurfaceError::WrongModel)
        );
        assert_eq!(last_outcome(&s), "refused:wrong_model");

        // A document that has replaced text cannot become append-only.
        log.conflict_model = ConflictModel::LastWriterWins;
        s.define_document(&owner(), log.clone(), T0).unwrap();
        s.edit_document(
            &r,
            &worker(1),
            "log",
            EditOp::Replace {
                text: "two".to_owned(),
            },
            T0,
        )
        .unwrap();
        log.conflict_model = ConflictModel::AppendOnly;
        assert_eq!(
            s.define_document(&owner(), log, T0),
            Err(SurfaceError::WrongModel)
        );
        // Configuration edits keep the content.
        assert_eq!(
            s.document(&r, &worker(1), "log", T0).unwrap().content,
            "two"
        );
    }

    #[test]
    fn subscribers_get_semantic_diffs_or_a_snapshot_with_the_same_authority() {
        let r = registry();
        let mut s = WorkSurfaces::with_retention(WORLD, 4);
        s.define_board(&owner(), BoardDefinition::starter("work"), T0)
            .unwrap();
        let since = s.revision();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let v = item(&mut s, &r, id).version;
        s.claim(&r, &worker(1), "work", id, v, T0).unwrap();

        let board = SurfaceId::Board("work".to_owned());
        let CatchUp::Diffs { through, changes } =
            s.catch_up(&r, &visitor(), &board, since, T0).unwrap()
        else {
            panic!("diffs expected while retained");
        };
        assert_eq!(through, s.revision());
        assert_eq!(
            changes.iter().map(|c| c.diff.clone()).collect::<Vec<_>>(),
            vec![
                SemanticDiff::ItemCreated { item: id },
                SemanticDiff::ItemClaimed {
                    item: id,
                    holder: worker(1),
                    epoch: 1
                },
            ]
        );
        // Nothing new: an empty diff.
        assert_eq!(
            s.catch_up(&r, &visitor(), &board, through, T0).unwrap(),
            CatchUp::Diffs {
                through,
                changes: Vec::new()
            }
        );

        // Push the early changes out of retention.
        for n in 0..4 {
            s.create_item(&r, &worker(2), "work", draft(&format!("more {n}")), T0)
                .unwrap();
        }
        let CatchUp::Snapshot { through, view } =
            s.catch_up(&r, &visitor(), &board, since, T0).unwrap()
        else {
            panic!("snapshot expected once pruned");
        };
        assert_eq!(through, s.revision());
        let SurfaceView::Board(view) = view else {
            panic!("board view expected");
        };
        assert_eq!(view.items.len(), 5);
        // The reduced-fidelity path is still refused to a role without read.
        let mut hidden = BoardDefinition::starter("work");
        hidden.roles.remove("observer");
        s.define_board(&owner(), hidden, T0).unwrap();
        assert_eq!(
            s.catch_up(&r, &visitor(), &board, since, T0),
            Err(SurfaceError::NotPermitted)
        );
    }

    #[test]
    fn every_mutation_is_audited() {
        let r = registry();
        let mut s = surfaces();
        let before = s.events().len();
        let id = s
            .create_item(&r, &worker(1), "work", draft("design"), T0)
            .unwrap();
        let v = item(&mut s, &r, id).version;
        s.claim(&r, &worker(1), "work", id, v, T0).unwrap();
        s.release(&r, &worker(1), "work", id, T0).unwrap();
        let kinds: Vec<&str> = s.events()[before..]
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            vec![EVENT_ITEM_CREATE, EVENT_ITEM_CLAIM, EVENT_ITEM_RELEASE]
        );
        assert!(
            s.events()[before..]
                .iter()
                .all(|e| e.outcome == "executed" && e.world == WORLD)
        );
    }

    #[test]
    fn bad_board_definitions_are_refused() {
        let mut s = WorkSurfaces::new(WORLD);
        let mut bad = BoardDefinition::starter("b");
        bad.states.clear();
        assert_eq!(
            s.define_board(&owner(), bad, T0),
            Err(SurfaceError::BadDefinition)
        );
        let mut bad = BoardDefinition::starter("b");
        bad.closing_states = vec!["todo".to_owned()];
        assert_eq!(
            s.define_board(&owner(), bad, T0),
            Err(SurfaceError::BadDefinition)
        );
        let mut bad = BoardDefinition::starter("b");
        bad.states.push("todo".to_owned());
        assert_eq!(
            s.define_board(&owner(), bad, T0),
            Err(SurfaceError::BadDefinition)
        );
        let mut bad = BoardDefinition::starter("b");
        bad.claim_lease_secs = 0;
        assert_eq!(
            s.define_board(&owner(), bad, T0),
            Err(SurfaceError::BadDefinition)
        );
        // Editing a good board bumps its version.
        assert_eq!(
            s.define_board(&owner(), BoardDefinition::starter("b"), T0),
            Ok(1)
        );
        assert_eq!(
            s.define_board(&owner(), BoardDefinition::starter("b"), T0),
            Ok(2)
        );
    }

    #[test]
    fn definitions_are_serde_shaped_world_configuration() {
        let board = BoardDefinition::starter("work");
        let json = serde_json::to_string(&board).unwrap();
        assert_eq!(
            serde_json::from_str::<BoardDefinition>(&json).unwrap(),
            board
        );
        assert!(
            serde_json::from_str::<BoardDefinition>(
                r#"{"name":"x","states":["a"],"claim_lease_secs":1,"extra":1}"#
            )
            .is_err()
        );
    }
}
