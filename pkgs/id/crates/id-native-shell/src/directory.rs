//! World directory for the native shell application (issue #15022,
//! graphical-client design #5782 §3): the first end-user application
//! surface on the F13.6 shell foundation.
//!
//! The directory is what the app opens on when it is launched at a
//! host without naming a world: the host's public world directory
//! (the existing discovery API, [`id::world_net::WorldClient::directory`])
//! rendered as one list — pinned worlds first, then recent, then
//! discovered, exactly the organisation design §3 prescribes. The
//! user navigates it by keyboard, opens a world into its session,
//! and leaves back to the directory; per §3, switching or leaving
//! never tears down another world's session, which the
//! [`SessionRegistry`] models and the binary's per-world session
//! bridges honour.
//!
//! Everything in this module is presentation-side state: the model
//! (ordering, cursor, marks, registry) is pure and headless-tested,
//! and the Bevy plugin renders the same lines the accessible text
//! table produces, so the graphical list and the list/table path
//! can never disagree about what is shown.

use std::collections::BTreeSet;
use std::time::Duration;

use bevy::input::ButtonInput;
use bevy::prelude::{
    App, Commands, Component, Entity, IntoScheduleConfigs, KeyCode, Local, Node, Plugin, Query,
    Res, Resource, Startup, SystemSet, Text, Update, With,
};
use serde_json::{Value, json};

/// How many worlds the recent section remembers. Recent is a
/// convenience ordering, not a history: older entries fall off the
/// end and simply rejoin the discovered section.
pub const MAX_RECENT_WORLDS: usize = 10;

/// How often the binary re-reads the host's world directory while
/// the app runs (design #5782 §3: the list is the discovery query
/// rendered, so it follows the host rather than freezing at
/// launch). The refresh key asks for a read at once; this interval
/// is the floor for everything else.
pub const DIRECTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Which section of the directory list a world sits in (design
/// #5782 §3: pinned first, then recent, then discovered).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldSection {
    /// The user pinned this world; pins keep the user's own order.
    Pinned,
    /// The user opened this world recently (most recent first).
    Recent,
    /// The host's directory reports this world; the user has not
    /// pinned or opened it.
    Discovered,
}

impl WorldSection {
    /// The bracketed tag the list shows for this section.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Pinned => "[pinned]",
            Self::Recent => "[recent]",
            Self::Discovered => "[discovered]",
        }
    }
}

/// One world in the directory list: the host directory's facts
/// (name, loaded state, presence count) plus the section the
/// user's marks place it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldEntry {
    /// World (routing) name.
    pub name: String,
    /// Whether the world is currently open in the host's memory.
    /// A stored world opens when it is selected; this is the
    /// directory's `loaded`, not an open-to-join claim (#812).
    pub loaded: bool,
    /// How many sessions are live on this world right now (a
    /// count only, never identities — #812).
    pub presence: usize,
    /// Whether this is the host's default world.
    pub is_default: bool,
    /// The section this entry renders in.
    pub section: WorldSection,
    /// Whether the host's latest directory reply lists this world.
    /// False only for a world kept on the list because it still
    /// has a live session: a refresh never takes a live session's
    /// row away, so the user can always return to it or close it.
    pub listed: bool,
}

/// The user's own ordering state for the directory: which worlds
/// are pinned, and which were opened most recently. Marks are
/// local client state (persisted beside the client key file by
/// the binary); they are never sent to the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryMarks {
    /// Pinned world names, in the order the user pinned them.
    pub pinned: Vec<String>,
    /// Recently opened world names, most recent first, capped at
    /// [`MAX_RECENT_WORLDS`].
    pub recent: Vec<String>,
}

impl DirectoryMarks {
    /// Pin `name` if it is unpinned, unpin it if it is pinned.
    /// Returns whether the world is pinned after the toggle.
    pub fn toggle_pin(&mut self, name: &str) -> bool {
        if let Some(index) = self.pinned.iter().position(|pinned| pinned == name) {
            self.pinned.remove(index);
            false
        } else {
            self.pinned.push(name.to_string());
            true
        }
    }

    /// Record that the user opened `name`: it becomes the most
    /// recent world, and the list is capped at
    /// [`MAX_RECENT_WORLDS`].
    pub fn touch_recent(&mut self, name: &str) {
        self.recent.retain(|recent| recent != name);
        self.recent.insert(0, name.to_string());
        self.recent.truncate(MAX_RECENT_WORLDS);
    }

    /// Serialize the marks for local persistence.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "pinned": self.pinned, "recent": self.recent })
    }

    /// Parse marks from their persisted JSON shape. This is the
    /// user's own local state file, never wire data, so parsing is
    /// deliberately forgiving: a missing or mistyped field reads
    /// as empty, non-string entries are skipped, duplicates keep
    /// their first position, and the recent list is capped. A
    /// corrupt marks file must never stop the app from opening.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        fn names(value: &Value, key: &str) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            if let Some(items) = value.get(key).and_then(Value::as_array) {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if !out.iter().any(|seen| seen == name) {
                            out.push(name.to_string());
                        }
                    }
                }
            }
            out
        }
        let mut marks = Self {
            pinned: names(value, "pinned"),
            recent: names(value, "recent"),
        };
        marks.recent.truncate(MAX_RECENT_WORLDS);
        marks
    }
}

/// The ordered world list plus its selection cursor. Built from
/// one host directory reply and the user's marks; [`Self::refresh`]
/// rebuilds the order when the marks change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldDirectory {
    default: String,
    listing: Vec<(String, bool, usize)>,
    /// Worlds kept on the list though the latest reply no longer
    /// lists them, because they have live sessions.
    retained: Vec<String>,
    entries: Vec<WorldEntry>,
    selected: usize,
}

impl WorldDirectory {
    /// Build the directory from a host directory reply — the
    /// `(default, [(name, open, presence)])` shape
    /// [`id::world_net::WorldClient::directory`] returns — ordered
    /// pinned, then recent, then discovered (design #5782 §3).
    /// The cursor starts on the host's default world when it is
    /// listed, otherwise on the first entry.
    #[must_use]
    pub fn from_listing(
        default: &str,
        listing: &[(String, bool, usize)],
        marks: &DirectoryMarks,
    ) -> Self {
        let mut directory = Self {
            default: default.to_string(),
            listing: listing.to_vec(),
            retained: Vec::new(),
            entries: Vec::new(),
            selected: 0,
        };
        directory.entries = directory.build_entries(marks);
        directory.selected = directory
            .entries
            .iter()
            .position(|entry| entry.is_default)
            .unwrap_or(0);
        directory
    }

    fn build_entries(&self, marks: &DirectoryMarks) -> Vec<WorldEntry> {
        let mut entries: Vec<WorldEntry> = Vec::with_capacity(self.listing.len());
        let mut placed: BTreeSet<&str> = BTreeSet::new();
        for (names, section) in [
            (&marks.pinned, WorldSection::Pinned),
            (&marks.recent, WorldSection::Recent),
        ] {
            for name in names {
                if placed.contains(name.as_str()) {
                    continue;
                }
                if let Some(entry) = self.entry_for(name, section) {
                    placed.insert(name.as_str());
                    entries.push(entry);
                }
            }
        }
        let listed = self.listing.iter().map(|(name, _, _)| name);
        for name in listed.chain(self.retained.iter()) {
            if placed.contains(name.as_str()) {
                continue;
            }
            if let Some(entry) = self.entry_for(name, WorldSection::Discovered) {
                placed.insert(name.as_str());
                entries.push(entry);
            }
        }
        entries
    }

    fn entry_for(&self, name: &str, section: WorldSection) -> Option<WorldEntry> {
        if let Some((_, loaded, presence)) =
            self.listing.iter().find(|(listed, _, _)| listed == name)
        {
            return Some(WorldEntry {
                name: name.to_string(),
                loaded: *loaded,
                presence: *presence,
                is_default: name == self.default,
                section,
                listed: true,
            });
        }
        // A retained world: the host no longer lists it, so the
        // directory claims nothing about it beyond the live session.
        self.retained
            .iter()
            .any(|kept| kept == name)
            .then(|| WorldEntry {
                name: name.to_string(),
                loaded: false,
                presence: 0,
                is_default: false,
                section,
                listed: false,
            })
    }

    /// Rebuild the section order after the marks changed, keeping
    /// the cursor on the same world when it is still listed.
    pub fn refresh(&mut self, marks: &DirectoryMarks) {
        let current = self.selected().map(|entry| entry.name.clone());
        self.entries = self.build_entries(marks);
        self.selected = current
            .and_then(|name| self.entries.iter().position(|entry| entry.name == name))
            .unwrap_or(0);
    }

    /// Fold a fresh host directory reply into the list (design
    /// #5782 §3, live refresh). Worlds named in `keep` (the live
    /// sessions) that the reply no longer lists stay on the list,
    /// marked as no longer listed. The cursor stays on the same
    /// world when it is still on the list; when it is gone the
    /// cursor stays on the same row, clamped to the new length,
    /// rather than jumping to the top.
    pub fn update_listing(
        &mut self,
        default: &str,
        listing: &[(String, bool, usize)],
        marks: &DirectoryMarks,
        keep: &BTreeSet<String>,
    ) {
        default.clone_into(&mut self.default);
        self.listing = listing.to_vec();
        self.retain_live(marks, keep);
    }

    /// Recompute which unlisted worlds stay on the list (those in
    /// `keep`) and rebuild the order, with the cursor rule of
    /// [`Self::update_listing`].
    pub fn retain_live(&mut self, marks: &DirectoryMarks, keep: &BTreeSet<String>) {
        let current = self.selected().map(|entry| entry.name.clone());
        let row = self.selected;
        self.retained = keep
            .iter()
            .filter(|name| !self.listing.iter().any(|(listed, _, _)| listed == *name))
            .cloned()
            .collect();
        self.entries = self.build_entries(marks);
        self.selected = current
            .and_then(|name| self.entries.iter().position(|entry| entry.name == name))
            .unwrap_or_else(|| row.min(self.entries.len().saturating_sub(1)));
    }

    /// The host's default world name from the directory reply.
    #[must_use]
    pub fn default_world(&self) -> &str {
        &self.default
    }

    /// Every entry, in display order.
    #[must_use]
    pub fn entries(&self) -> &[WorldEntry] {
        &self.entries
    }

    /// The entry under the cursor, if the list is non-empty.
    #[must_use]
    pub fn selected(&self) -> Option<&WorldEntry> {
        self.entries.get(self.selected)
    }

    /// Move the cursor one entry up, stopping at the first entry.
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the cursor one entry down, stopping at the last entry.
    pub fn move_down(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }

    /// Move the cursor to the first entry.
    pub fn move_home(&mut self) {
        self.selected = 0;
    }

    /// Move the cursor to the last entry.
    pub fn move_end(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
    }

    /// Move the cursor to `name`; returns whether it was listed.
    pub fn select(&mut self, name: &str) -> bool {
        if let Some(index) = self.entries.iter().position(|entry| entry.name == name) {
            self.selected = index;
            true
        } else {
            false
        }
    }
}

/// Which worlds have live sessions, and which one is in the
/// foreground. This is the state half of design #5782 §3's rule
/// that one client holds many worlds' sessions at once: entering
/// a world foregrounds it, leaving returns to the directory, and
/// neither act removes any other world's session — only an
/// explicit close does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionRegistry {
    live: BTreeSet<String>,
    foreground: Option<String>,
}

impl SessionRegistry {
    /// Enter `name`'s session: mark it live and foreground it.
    /// Returns whether the session was newly opened (false when
    /// an already-live session is merely foregrounded again).
    /// Every other live session is untouched.
    pub fn enter(&mut self, name: &str) -> bool {
        let newly = self.live.insert(name.to_string());
        self.foreground = Some(name.to_string());
        newly
    }

    /// Leave the foreground session back to the directory. The
    /// session — and every other — stays live.
    pub fn leave(&mut self) {
        self.foreground = None;
    }

    /// Close `name`'s session, and only that one. Returns whether
    /// it was live. Closing the foreground session returns the
    /// foreground to the directory.
    pub fn close(&mut self, name: &str) -> bool {
        let was_live = self.live.remove(name);
        if self.foreground.as_deref() == Some(name) {
            self.foreground = None;
        }
        was_live
    }

    /// Whether `name` has a live session.
    #[must_use]
    pub fn is_live(&self, name: &str) -> bool {
        self.live.contains(name)
    }

    /// The foreground world's name, if a session is foregrounded.
    #[must_use]
    pub fn foreground(&self) -> Option<&str> {
        self.foreground.as_deref()
    }

    /// Every live session's world name, in sorted order.
    pub fn live(&self) -> impl Iterator<Item = &str> {
        self.live.iter().map(String::as_str)
    }
}

/// One keypress, as the directory understands it. The Bevy
/// keyboard system translates `KeyCode`s into these; tests drive
/// the model with them directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryKey {
    /// Cursor up.
    Up,
    /// Cursor down.
    Down,
    /// Cursor to the first entry.
    Home,
    /// Cursor to the last entry.
    End,
    /// Open the selected world (or foreground its live session).
    Activate,
    /// Leave the foreground session back to the directory.
    Leave,
    /// Pin or unpin the selected world.
    TogglePin,
    /// Close the selected world's session (only that one).
    Close,
    /// Re-read the host's world directory now.
    Refresh,
}

/// A side effect the application shell must perform on the
/// model's behalf: the model owns no connections, so opening and
/// closing sessions are handed to the binary as effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryEffect {
    /// Start (or foreground) the named world's session bridge.
    EnterWorld(String),
    /// Tear down the named world's session bridge — only that one.
    CloseWorld(String),
    /// The marks changed; persist them.
    MarksChanged,
    /// Re-read the host's world directory. Queued at most once
    /// until the binary takes the effects.
    RefreshDirectory,
}

/// Whether the user's attention is on the directory or inside
/// the foreground session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectoryFocus {
    /// Browsing the world list.
    #[default]
    Directory,
    /// Inside the foreground session.
    Session,
}

/// The directory application state: marks, the current listing,
/// the session registry, the focus, and the queue of effects the
/// binary has not performed yet.
#[derive(Resource, Clone, Debug, Default)]
pub struct DirectoryApp {
    /// The user's pins and recents.
    pub marks: DirectoryMarks,
    /// Which worlds have live sessions, and the foreground one.
    pub registry: SessionRegistry,
    /// Directory or session focus.
    pub focus: DirectoryFocus,
    directory: Option<WorldDirectory>,
    effects: Vec<DirectoryEffect>,
}

impl DirectoryApp {
    /// Directory state with the given persisted marks.
    #[must_use]
    pub fn with_marks(marks: DirectoryMarks) -> Self {
        Self {
            marks,
            ..Self::default()
        }
    }

    /// Record a host directory reply. The first builds the list
    /// with the cursor on the host's default world; every later
    /// one is a refresh, folded in by
    /// [`WorldDirectory::update_listing`] so the cursor stays put
    /// and no live session's row is dropped.
    pub fn set_listing(&mut self, default: &str, listing: &[(String, bool, usize)]) {
        let live: BTreeSet<String> = self.registry.live().map(str::to_owned).collect();
        match self.directory.as_mut() {
            Some(directory) => directory.update_listing(default, listing, &self.marks, &live),
            None => {
                self.directory = Some(WorldDirectory::from_listing(default, listing, &self.marks));
            }
        }
    }

    /// The current directory, once a listing has arrived.
    #[must_use]
    pub fn directory(&self) -> Option<&WorldDirectory> {
        self.directory.as_ref()
    }

    /// Handle one keypress against the current focus. In session
    /// focus only Leave is the directory's business (it returns
    /// to the directory with every session still live); all other
    /// keys belong to the session's own presenters.
    pub fn handle_key(&mut self, key: DirectoryKey) {
        // Refresh needs no listing yet: it is also how the user
        // retries after the first read failed.
        if key == DirectoryKey::Refresh {
            if self.focus == DirectoryFocus::Directory
                && !self.effects.contains(&DirectoryEffect::RefreshDirectory)
            {
                self.effects.push(DirectoryEffect::RefreshDirectory);
            }
            return;
        }
        match self.focus {
            DirectoryFocus::Session => {
                if key == DirectoryKey::Leave {
                    self.registry.leave();
                    self.focus = DirectoryFocus::Directory;
                }
            }
            DirectoryFocus::Directory => self.handle_directory_key(key),
        }
    }

    fn handle_directory_key(&mut self, key: DirectoryKey) {
        let Some(directory) = self.directory.as_mut() else {
            return;
        };
        match key {
            DirectoryKey::Up => directory.move_up(),
            DirectoryKey::Down => directory.move_down(),
            DirectoryKey::Home => directory.move_home(),
            DirectoryKey::End => directory.move_end(),
            DirectoryKey::Leave | DirectoryKey::Refresh => {}
            DirectoryKey::TogglePin => {
                if let Some(name) = directory.selected().map(|entry| entry.name.clone()) {
                    self.marks.toggle_pin(&name);
                    directory.refresh(&self.marks);
                    self.effects.push(DirectoryEffect::MarksChanged);
                }
            }
            DirectoryKey::Activate => {
                if let Some(name) = directory.selected().map(|entry| entry.name.clone()) {
                    self.marks.touch_recent(&name);
                    directory.refresh(&self.marks);
                    self.registry.enter(&name);
                    self.focus = DirectoryFocus::Session;
                    self.effects.push(DirectoryEffect::MarksChanged);
                    self.effects.push(DirectoryEffect::EnterWorld(name));
                }
            }
            DirectoryKey::Close => {
                if let Some(name) = directory.selected().map(|entry| entry.name.clone()) {
                    if self.registry.close(&name) {
                        // A closed world the host no longer lists
                        // leaves the list with its session.
                        let live: BTreeSet<String> =
                            self.registry.live().map(str::to_owned).collect();
                        directory.retain_live(&self.marks, &live);
                        self.effects.push(DirectoryEffect::CloseWorld(name));
                    }
                }
            }
        }
    }

    /// Where a key press in the native window goes when it is not
    /// the directory's (#15022, session-mode input): the
    /// foreground world's name while a session has focus, so the
    /// key reaches that world's session as terminal input. Nothing
    /// goes anywhere while the directory has focus, and the leave
    /// key (Esc) never reaches the world — it is how the user gets
    /// back to this list.
    #[must_use]
    pub fn session_input_target(&self, is_leave_key: bool) -> Option<&str> {
        if self.focus != DirectoryFocus::Session || is_leave_key {
            return None;
        }
        self.registry.foreground()
    }

    /// Take the effects queued since the last call, in order.
    pub fn take_effects(&mut self) -> Vec<DirectoryEffect> {
        std::mem::take(&mut self.effects)
    }

    /// The accessible list/table projection of the directory:
    /// one header line, one row per world (cursor, default star,
    /// section tag, name, loaded state, presence, live-session
    /// mark), and a key hint line. The Bevy plugin renders exactly
    /// these lines, so the graphical list and this projection are
    /// the same list by construction.
    #[must_use]
    pub fn text_table(&self) -> String {
        let Some(directory) = self.directory.as_ref() else {
            return "World directory\nWaiting for the host world list. R retries now.".to_string();
        };
        let mut lines = vec![format!(
            "World directory - default: {}",
            directory.default_world()
        )];
        if directory.entries().is_empty() {
            lines.push("No worlds on this host yet.".to_string());
        }
        let name_width = directory
            .entries()
            .iter()
            .map(|entry| entry.name.len())
            .max()
            .unwrap_or(0);
        let selected = directory.selected().map(|entry| entry.name.clone());
        for entry in directory.entries() {
            let cursor = if selected.as_deref() == Some(entry.name.as_str()) {
                ">"
            } else {
                " "
            };
            let star = if entry.is_default { "*" } else { " " };
            let state = match (entry.listed, entry.loaded) {
                (false, _) => "no longer listed",
                (true, true) => "loaded",
                (true, false) => "stored",
            };
            let session = if self.registry.is_live(&entry.name) {
                " (in session)"
            } else {
                ""
            };
            lines.push(format!(
                "{cursor}{star} {:<12} {:<width$}  {state}  {} here{session}",
                entry.section.tag(),
                entry.name,
                entry.presence,
                width = name_width,
            ));
        }
        lines.push(
            "Up/Down move, Enter open, P pin, X close session, R refresh, Esc back to this list."
                .to_string(),
        );
        lines.join("\n")
    }
}

/// Translate a Bevy key code into a directory key, if the
/// directory uses it.
#[must_use]
pub const fn key_code_to_directory_key(code: KeyCode) -> Option<DirectoryKey> {
    match code {
        KeyCode::ArrowUp => Some(DirectoryKey::Up),
        KeyCode::ArrowDown => Some(DirectoryKey::Down),
        KeyCode::Home => Some(DirectoryKey::Home),
        KeyCode::End => Some(DirectoryKey::End),
        KeyCode::Enter => Some(DirectoryKey::Activate),
        KeyCode::Escape => Some(DirectoryKey::Leave),
        KeyCode::KeyP => Some(DirectoryKey::TogglePin),
        KeyCode::KeyX => Some(DirectoryKey::Close),
        KeyCode::KeyR | KeyCode::F5 => Some(DirectoryKey::Refresh),
        _ => None,
    }
}

/// The system set that feeds key presses to [`DirectoryApp`].
/// Session-mode input forwarding runs before it, so a key that
/// changes focus (Enter into a world, Esc out of one) is judged by
/// the focus it was pressed in, never sent to a world it opened.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectoryInputSystems;

/// Marker for the root UI node the directory presenter owns.
#[derive(Component)]
struct DirectoryRoot;

/// Marker for one rendered directory row node.
#[derive(Component)]
struct DirectoryRow;

/// Bevy plugin: presents [`DirectoryApp`] as a UI node tree while
/// the directory has focus, and routes keyboard input to it.
/// Register after Bevy's UI/text plugins (the binary uses
/// `DefaultPlugins`). The systems only spawn plain `Node`/`Text`
/// components and read `ButtonInput` when it exists, so tests can
/// run the plugin under `MinimalPlugins` and inspect the spawned
/// tree headlessly.
#[derive(Debug)]
pub struct DirectoryPlugin;

impl Plugin for DirectoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DirectoryApp>()
            .add_systems(Startup, spawn_directory_root)
            .add_systems(
                Update,
                (
                    directory_keyboard.in_set(DirectoryInputSystems),
                    rebuild_directory_rows,
                ),
            );
    }
}

fn spawn_directory_root(mut commands: Commands<'_, '_>) {
    commands.spawn((
        DirectoryRoot,
        Node {
            flex_direction: bevy::prelude::FlexDirection::Column,
            width: bevy::prelude::Val::Percent(100.0),
            ..Default::default()
        },
    ));
}

/// Feed this frame's keypresses to the directory model.
#[allow(clippy::needless_pass_by_value)]
fn directory_keyboard(
    keys: Option<Res<'_, ButtonInput<KeyCode>>>,
    mut directory: bevy::prelude::ResMut<'_, DirectoryApp>,
) {
    let Some(keys) = keys else {
        return;
    };
    for code in keys.get_just_pressed() {
        if let Some(key) = key_code_to_directory_key(*code) {
            directory.handle_key(key);
        }
    }
}

/// Rebuild the directory rows when the rendered table changes.
/// In session focus the directory renders nothing: the session's
/// own presenters own the window.
#[allow(clippy::needless_pass_by_value)]
fn rebuild_directory_rows(
    directory: bevy::prelude::Res<'_, DirectoryApp>,
    mut last_table: Local<'_, Option<String>>,
    roots: Query<'_, '_, Entity, With<DirectoryRoot>>,
    rows: Query<'_, '_, Entity, With<DirectoryRow>>,
    mut commands: Commands<'_, '_>,
) {
    let table = match directory.focus {
        DirectoryFocus::Directory => Some(directory.text_table()),
        DirectoryFocus::Session => None,
    };
    if table == *last_table {
        return;
    }
    *last_table = table.clone();
    for entity in &rows {
        commands.entity(entity).despawn();
    }
    let (Ok(root), Some(table)) = (roots.single(), table) else {
        return;
    };
    for line in table.lines() {
        let row = commands
            .spawn((DirectoryRow, Node::default(), Text::new(line)))
            .id();
        commands.entity(root).add_child(row);
    }
}

/// Build a headless app with only the directory plugin (tests,
/// tools).
#[must_use]
pub fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::prelude::MinimalPlugins)
        .add_plugins(DirectoryPlugin);
    app
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn listing() -> Vec<(String, bool, usize)> {
        vec![
            ("annex".to_string(), true, 1),
            ("lobby".to_string(), true, 2),
            ("workshop".to_string(), false, 0),
            ("vault".to_string(), false, 0),
        ]
    }

    #[test]
    fn directory_orders_pinned_then_recent_then_discovered() {
        let mut marks = DirectoryMarks::default();
        marks.toggle_pin("vault");
        marks.toggle_pin("annex");
        marks.touch_recent("workshop");
        marks.touch_recent("lobby");
        let directory = WorldDirectory::from_listing("lobby", &listing(), &marks);
        let names: Vec<(&str, WorldSection)> = directory
            .entries()
            .iter()
            .map(|entry| (entry.name.as_str(), entry.section))
            .collect();
        // Oracle: design #5782 §3 — pinned in pin order, then
        // recent most-recent-first, then discovered in host order;
        // every listed world appears exactly once.
        assert_eq!(
            names,
            vec![
                ("vault", WorldSection::Pinned),
                ("annex", WorldSection::Pinned),
                ("lobby", WorldSection::Recent),
                ("workshop", WorldSection::Recent),
            ]
        );
        assert!(directory.entries()[2].is_default);
        assert_eq!(directory.default_world(), "lobby");
    }

    #[test]
    fn marks_for_unlisted_worlds_do_not_fabricate_entries() {
        let mut marks = DirectoryMarks::default();
        marks.toggle_pin("elsewhere");
        marks.touch_recent("elsewhere");
        let directory = WorldDirectory::from_listing("lobby", &listing(), &marks);
        assert_eq!(directory.entries().len(), 4);
        assert!(
            directory
                .entries()
                .iter()
                .all(|entry| entry.section == WorldSection::Discovered)
        );
    }

    #[test]
    fn cursor_starts_on_default_and_clamps_at_ends() {
        let directory_marks = DirectoryMarks::default();
        let mut directory = WorldDirectory::from_listing("lobby", &listing(), &directory_marks);
        // Listing order: annex, lobby, workshop, vault; the
        // default (lobby) is selected first.
        assert_eq!(directory.selected().unwrap().name, "lobby");
        directory.move_up();
        assert_eq!(directory.selected().unwrap().name, "annex");
        directory.move_up();
        assert_eq!(directory.selected().unwrap().name, "annex");
        directory.move_end();
        assert_eq!(directory.selected().unwrap().name, "vault");
        directory.move_down();
        assert_eq!(directory.selected().unwrap().name, "vault");
        directory.move_home();
        assert_eq!(directory.selected().unwrap().name, "annex");
        assert!(directory.select("workshop"));
        assert_eq!(directory.selected().unwrap().name, "workshop");
        assert!(!directory.select("missing"));
    }

    #[test]
    fn empty_directory_has_no_selection_and_safe_navigation() {
        let mut directory = WorldDirectory::from_listing("none", &[], &DirectoryMarks::default());
        assert!(directory.selected().is_none());
        directory.move_down();
        directory.move_end();
        assert!(directory.selected().is_none());
    }

    #[test]
    fn recent_list_dedupes_and_caps() {
        let mut marks = DirectoryMarks::default();
        for index in 0..(MAX_RECENT_WORLDS + 3) {
            marks.touch_recent(&format!("world-{index}"));
        }
        assert_eq!(marks.recent.len(), MAX_RECENT_WORLDS);
        assert_eq!(marks.recent[0], format!("world-{}", MAX_RECENT_WORLDS + 2));
        marks.touch_recent("world-5");
        assert_eq!(marks.recent[0], "world-5");
        assert_eq!(
            marks
                .recent
                .iter()
                .filter(|name| name.as_str() == "world-5")
                .count(),
            1
        );
    }

    #[test]
    fn marks_json_round_trips_and_forgives_damage() {
        let mut marks = DirectoryMarks::default();
        marks.toggle_pin("lobby");
        marks.touch_recent("annex");
        let parsed = DirectoryMarks::from_json(&marks.to_json());
        assert_eq!(parsed, marks);
        // Oracle: local-state parsing never fails the app — a
        // damaged file reads as the salvageable subset.
        let damaged = DirectoryMarks::from_json(&json!({
            "pinned": ["a", 7, "a", "b"],
            "recent": "not-a-list",
        }));
        assert_eq!(damaged.pinned, vec!["a".to_string(), "b".to_string()]);
        assert!(damaged.recent.is_empty());
        assert_eq!(
            DirectoryMarks::from_json(&Value::Null),
            DirectoryMarks::default()
        );
    }

    #[test]
    fn registry_enter_and_leave_never_tear_down_other_sessions() {
        let mut registry = SessionRegistry::default();
        assert!(registry.enter("lobby"));
        assert!(registry.enter("annex"));
        assert_eq!(registry.foreground(), Some("annex"));
        // Re-entering a live session foregrounds it, opens nothing.
        assert!(!registry.enter("lobby"));
        registry.leave();
        assert_eq!(registry.foreground(), None);
        // Oracle: design #5782 §3 — after entering two worlds and
        // leaving to the directory, both sessions are still live.
        assert!(registry.is_live("lobby"));
        assert!(registry.is_live("annex"));
        assert_eq!(registry.live().collect::<Vec<_>>(), vec!["annex", "lobby"]);
        // Closing one session removes only that one.
        assert!(registry.close("lobby"));
        assert!(!registry.is_live("lobby"));
        assert!(registry.is_live("annex"));
        assert!(!registry.close("lobby"));
    }

    #[test]
    fn closing_the_foreground_session_clears_the_foreground() {
        let mut registry = SessionRegistry::default();
        registry.enter("lobby");
        registry.enter("annex");
        assert!(registry.close("annex"));
        assert_eq!(registry.foreground(), None);
        assert!(registry.is_live("lobby"));
    }

    #[test]
    fn app_activate_enters_session_and_records_recent() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        app.handle_key(DirectoryKey::Activate);
        assert_eq!(app.focus, DirectoryFocus::Session);
        assert_eq!(app.registry.foreground(), Some("lobby"));
        assert_eq!(app.marks.recent, vec!["lobby".to_string()]);
        assert_eq!(
            app.take_effects(),
            vec![
                DirectoryEffect::MarksChanged,
                DirectoryEffect::EnterWorld("lobby".to_string()),
            ]
        );
        // In session focus, navigation keys are not the
        // directory's business; Leave returns to the list with
        // the session still live.
        app.handle_key(DirectoryKey::Down);
        assert_eq!(app.registry.foreground(), Some("lobby"));
        app.handle_key(DirectoryKey::Leave);
        assert_eq!(app.focus, DirectoryFocus::Directory);
        assert!(app.registry.is_live("lobby"));
        assert!(app.take_effects().is_empty());
    }

    #[test]
    fn app_pin_toggle_reorders_and_keeps_cursor_on_the_world() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        // Cursor starts on the default (lobby); move up to annex
        // and pin it: it jumps to the pinned section, cursor follows.
        app.handle_key(DirectoryKey::Up);
        assert_eq!(app.directory().unwrap().selected().unwrap().name, "annex");
        app.handle_key(DirectoryKey::TogglePin);
        let directory = app.directory().unwrap();
        assert_eq!(directory.entries()[0].name, "annex");
        assert_eq!(directory.entries()[0].section, WorldSection::Pinned);
        assert_eq!(directory.selected().unwrap().name, "annex");
        assert_eq!(app.take_effects(), vec![DirectoryEffect::MarksChanged]);
        // Unpinning returns it to discovered, still under the cursor.
        app.handle_key(DirectoryKey::TogglePin);
        let directory = app.directory().unwrap();
        assert_eq!(directory.selected().unwrap().name, "annex");
        assert_eq!(
            directory.selected().unwrap().section,
            WorldSection::Discovered
        );
    }

    #[test]
    fn app_close_only_closes_the_selected_worlds_session() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        app.registry.enter("lobby");
        app.registry.enter("annex");
        app.registry.leave();
        // The cursor starts on the default world, lobby.
        app.handle_key(DirectoryKey::Close);
        assert_eq!(
            app.take_effects(),
            vec![DirectoryEffect::CloseWorld("lobby".to_string())]
        );
        assert!(!app.registry.is_live("lobby"));
        assert!(app.registry.is_live("annex"));
        // Closing a world with no live session is a no-op.
        app.handle_key(DirectoryKey::Close);
        assert!(app.take_effects().is_empty());
    }

    #[test]
    fn text_table_is_the_accessible_projection_of_the_list() {
        let mut app = DirectoryApp::default();
        assert!(
            app.text_table()
                .contains("Waiting for the host world list.")
        );
        app.set_listing("lobby", &listing());
        app.registry.enter("annex");
        let table = app.text_table();
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "World directory - default: lobby");
        // One row per world plus header and hint lines.
        assert_eq!(lines.len(), 1 + 4 + 1);
        // Rows are lines[1..]; find each world's row by its name
        // as a whole word (the header also names the default).
        let row_of = |name: &str| {
            lines[1..]
                .iter()
                .find(|line| line.split_whitespace().any(|word| word == name))
                .copied()
                .unwrap()
        };
        // The cursor row is the default world (lobby), starred.
        let lobby_row = row_of("lobby");
        assert!(lobby_row.starts_with(">*"), "row: {lobby_row}");
        assert!(lobby_row.contains("[discovered]"));
        assert!(lobby_row.contains("loaded"));
        assert!(lobby_row.contains("2 here"));
        let annex_row = row_of("annex");
        assert!(annex_row.contains("(in session)"), "row: {annex_row}");
        let workshop_row = row_of("workshop");
        assert!(workshop_row.contains("stored"));
        assert!(workshop_row.contains("0 here"));
    }

    #[test]
    fn key_code_translation_covers_the_directory_keys() {
        assert_eq!(
            key_code_to_directory_key(KeyCode::ArrowUp),
            Some(DirectoryKey::Up)
        );
        assert_eq!(
            key_code_to_directory_key(KeyCode::Enter),
            Some(DirectoryKey::Activate)
        );
        assert_eq!(
            key_code_to_directory_key(KeyCode::Escape),
            Some(DirectoryKey::Leave)
        );
        assert_eq!(
            key_code_to_directory_key(KeyCode::KeyR),
            Some(DirectoryKey::Refresh)
        );
        assert_eq!(
            key_code_to_directory_key(KeyCode::F5),
            Some(DirectoryKey::Refresh)
        );
        assert_eq!(key_code_to_directory_key(KeyCode::KeyA), None);
    }

    fn without(name: &str) -> Vec<(String, bool, usize)> {
        listing()
            .into_iter()
            .filter(|(listed, _, _)| listed != name)
            .collect()
    }

    #[test]
    fn refresh_keeps_the_cursor_on_the_same_world() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        app.handle_key(DirectoryKey::End);
        assert_eq!(app.directory().unwrap().selected().unwrap().name, "vault");
        // Oracle: #15022 — a refresh that reorders the rows (a new
        // world ahead of the cursor) leaves the cursor on vault.
        let mut grown = vec![("arena".to_string(), true, 3)];
        grown.extend(listing());
        app.set_listing("lobby", &grown);
        let directory = app.directory().unwrap();
        assert_eq!(directory.entries().len(), 5);
        assert_eq!(directory.entries()[0].name, "arena");
        assert_eq!(directory.selected().unwrap().name, "vault");
        // Presence and loaded state come from the newest reply.
        let mut busier = listing();
        busier[1].2 = 9;
        app.set_listing("lobby", &busier);
        let lobby = app
            .directory()
            .unwrap()
            .entries()
            .iter()
            .find(|entry| entry.name == "lobby")
            .unwrap()
            .clone();
        assert_eq!(lobby.presence, 9);
        assert!(lobby.listed);
    }

    #[test]
    fn refresh_clamps_the_cursor_when_its_world_disappears() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        app.handle_key(DirectoryKey::End);
        // The last row (vault) goes away: the cursor stays on the
        // last row rather than jumping to the top.
        app.set_listing("lobby", &without("vault"));
        let directory = app.directory().unwrap();
        assert_eq!(directory.entries().len(), 3);
        assert_eq!(directory.selected().unwrap().name, "workshop");
        // A middle row going away keeps the same row index.
        app.handle_key(DirectoryKey::Home);
        app.handle_key(DirectoryKey::Down);
        assert_eq!(app.directory().unwrap().selected().unwrap().name, "lobby");
        app.set_listing("annex", &without("lobby"));
        assert_eq!(
            app.directory().unwrap().selected().unwrap().name,
            "workshop"
        );
        // An empty reply leaves no selection and navigation safe.
        app.set_listing("none", &[]);
        assert!(app.directory().unwrap().selected().is_none());
        app.handle_key(DirectoryKey::Down);
        assert!(app.directory().unwrap().selected().is_none());
    }

    #[test]
    fn refresh_keeps_a_live_world_the_host_stopped_listing() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        app.handle_key(DirectoryKey::Activate);
        app.handle_key(DirectoryKey::Leave);
        app.take_effects();
        // Oracle: #15022 — a refresh never drops a world that has
        // a live session; it stays, marked no longer listed, and
        // the session is untouched.
        app.set_listing("annex", &without("lobby"));
        assert!(app.take_effects().is_empty());
        assert!(app.registry.is_live("lobby"));
        let directory = app.directory().unwrap();
        assert_eq!(directory.entries().len(), 4);
        let lobby = directory.selected().unwrap();
        assert_eq!(lobby.name, "lobby");
        assert!(!lobby.listed);
        assert!(!lobby.is_default);
        assert_eq!(lobby.presence, 0);
        let row = app
            .text_table()
            .lines()
            .find(|line| line.split_whitespace().any(|word| word == "lobby"))
            .unwrap()
            .to_string();
        assert!(row.contains("no longer listed"), "row: {row}");
        assert!(row.contains("(in session)"), "row: {row}");
        // Closing its session takes the unlisted world off the list.
        app.handle_key(DirectoryKey::Close);
        assert_eq!(
            app.take_effects(),
            vec![DirectoryEffect::CloseWorld("lobby".to_string())]
        );
        let directory = app.directory().unwrap();
        assert_eq!(directory.entries().len(), 3);
        assert!(
            directory
                .entries()
                .iter()
                .all(|entry| entry.name != "lobby")
        );
        assert!(directory.selected().is_some());
        // A world without a session that the host stops listing
        // simply leaves the list; one it lists again comes back
        // as listed.
        app.set_listing("annex", &without("vault"));
        let directory = app.directory().unwrap();
        assert!(
            directory
                .entries()
                .iter()
                .all(|entry| entry.name != "vault")
        );
        assert!(
            directory
                .entries()
                .iter()
                .any(|entry| entry.name == "lobby" && entry.listed)
        );
    }

    #[test]
    fn refresh_picks_up_a_new_default_world() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        app.set_listing("annex", &listing());
        let directory = app.directory().unwrap();
        assert_eq!(directory.default_world(), "annex");
        let defaults: Vec<&str> = directory
            .entries()
            .iter()
            .filter(|entry| entry.is_default)
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(defaults, vec!["annex"]);
        // The cursor did not follow the default: a refresh never
        // moves the user's place.
        assert_eq!(directory.selected().unwrap().name, "lobby");
    }

    #[test]
    fn refresh_key_queues_one_request_and_only_in_the_directory() {
        let mut app = DirectoryApp::default();
        // Before any listing: R is the retry for a failed first read.
        app.handle_key(DirectoryKey::Refresh);
        app.handle_key(DirectoryKey::Refresh);
        assert_eq!(app.take_effects(), vec![DirectoryEffect::RefreshDirectory]);
        app.set_listing("lobby", &listing());
        app.handle_key(DirectoryKey::Refresh);
        assert_eq!(app.take_effects(), vec![DirectoryEffect::RefreshDirectory]);
        // In a session, R belongs to the world, not the directory.
        app.handle_key(DirectoryKey::Activate);
        app.take_effects();
        app.handle_key(DirectoryKey::Refresh);
        assert!(app.take_effects().is_empty());
        assert_eq!(app.focus, DirectoryFocus::Session);
    }

    #[test]
    fn session_input_goes_only_to_the_foreground_world() {
        let mut app = DirectoryApp::default();
        app.set_listing("lobby", &listing());
        // Directory focus: keys are the directory's, none reach a world.
        assert_eq!(app.session_input_target(false), None);
        app.handle_key(DirectoryKey::Activate);
        // Oracle: #15022 — in a session, keys reach the foreground
        // world, except the leave key, which stays the way back.
        assert_eq!(app.session_input_target(false), Some("lobby"));
        assert_eq!(app.session_input_target(true), None);
        app.handle_key(DirectoryKey::Leave);
        assert_eq!(app.session_input_target(false), None);
        // A second world entered becomes the only target; the first
        // stays live in the background without input. (Lobby is
        // now the recent row on top; annex is the next row.)
        app.handle_key(DirectoryKey::Down);
        app.handle_key(DirectoryKey::Activate);
        assert_eq!(app.session_input_target(false), Some("annex"));
        assert!(app.registry.is_live("lobby"));
    }

    #[test]
    fn headless_plugin_renders_the_table_and_hides_it_in_session() {
        let mut app = headless_app();
        app.world_mut()
            .resource_mut::<DirectoryApp>()
            .set_listing("lobby", &listing());
        app.update();
        let mut query = app.world_mut().query::<(&DirectoryRow, &Text)>();
        let rows: Vec<String> = query
            .iter(app.world())
            .map(|(_, text)| text.0.clone())
            .collect();
        // Oracle: the graphical rows are exactly the accessible
        // table's lines, in order.
        let table = app.world().resource::<DirectoryApp>().text_table();
        assert_eq!(rows, table.lines().map(str::to_string).collect::<Vec<_>>());
        // Entering a session hides the directory rows.
        app.world_mut()
            .resource_mut::<DirectoryApp>()
            .handle_key(DirectoryKey::Activate);
        app.update();
        let mut query = app.world_mut().query::<(&DirectoryRow, &Text)>();
        assert_eq!(query.iter(app.world()).count(), 0);
    }
}
