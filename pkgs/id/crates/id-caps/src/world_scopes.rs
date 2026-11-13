//! Permission scopes granted by a world capability: the scope
//! vocabulary shared by the world runtime and the directory.
//!
//! Moved here from `id-worlds` (`world::WorldScopes`) as step 6a of
//! the pkgs/id crate split (#3447 umbrella, #4409): `id-directory`
//! names this type in its ceiling surface, and the directory must
//! not depend on the worlds crate. `id-worlds` re-exports it at its
//! historical `world::WorldScopes` path, so no call site changed.
//! `allows_effect` reads the provider registry in
//! [`crate::world_caps`], this crate's own catalog module.

/// Permission scopes granted by a world capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldScopes(u8);

impl WorldScopes {
    /// Join the world and receive snapshots/events.
    pub const JOIN: Self = Self(0b001);
    /// Send chat messages.
    pub const CHAT: Self = Self(0b010);
    /// Submit game input events.
    pub const INPUT: Self = Self(0b100);
    /// Mint capabilities with a subset of this set.
    pub const DELEGATE: Self = Self(0b1000);
    /// Read chat lines without being able to send them. `CHAT` also reads.
    pub const HISTORY: Self = Self(0b1_0000);
    /// Install and upgrade the world's program. Carries no capability,
    /// network, file or process authority: only the install path reads it.
    pub const BUILDER: Self = Self(0b10_0000);
    /// Revoke participants. Carries no capability, network, file or process
    /// authority: only the revoke path reads it.
    pub const MODERATOR: Self = Self(0b100_0000);
    /// No scopes at all: the empty set.
    pub const EMPTY: Self = Self(0);

    /// Default guest permissions.
    pub const GUEST: Self = Self(Self::JOIN.0 | Self::CHAT.0 | Self::INPUT.0);
    /// Every participant scope. The admin authority bits `BUILDER` and
    /// `MODERATOR` are deliberately excluded: they are issued only by the
    /// admin, never implied by a ceiling or handed out by a delegator.
    pub const ALL: Self = Self(Self::GUEST.0 | Self::DELEGATE.0 | Self::HISTORY.0);
    /// Every bit the type recognizes, including the admin authority bits.
    /// [`Self::from_bits`] accepts these so a journaled builder or moderator
    /// capability round-trips, without widening [`Self::ALL`].
    const ALL_BITS: u8 = Self::ALL.0 | Self::BUILDER.0 | Self::MODERATOR.0;

    /// Every recognized bit, including the admin authority bits. Used as
    /// the seed for authority-aware effective-scope computation: unlike
    /// [`Self::ALL`], this lets an admin-issued `BUILDER` / `MODERATOR`
    /// bit survive the ancestor-chain intersection in
    /// `WorldCore::authorize` (design #835 phase 2). It grants nothing on
    /// its own — the chain intersection only ever removes bits.
    pub const fn all_recognized() -> Self {
        Self(Self::ALL_BITS)
    }

    /// Whether this set includes the builder authority (install modules).
    #[must_use]
    pub const fn is_builder(self) -> bool {
        self.contains(Self::BUILDER)
    }

    /// Whether this set includes the moderator authority (revoke participants).
    #[must_use]
    pub const fn is_moderator(self) -> bool {
        self.contains(Self::MODERATOR)
    }

    /// Role presets used by `id world invite --role`. `spectator` joins and
    /// sees the world but not the chat; `observer` also reads chat without
    /// sending it; `player` submits input; `guest` (the default) also chats;
    /// `delegator` additionally mints narrower capabilities for others;
    /// `builder` additionally installs modules; `moderator` additionally
    /// revokes participants.
    ///
    /// The names are stable; the mapping to scopes is part of the protocol.
    #[must_use]
    pub fn from_role(name: &str) -> Option<Self> {
        match name {
            "spectator" => Some(Self::JOIN),
            "observer" => Some(Self(Self::JOIN.0 | Self::HISTORY.0)),
            "player" => Some(Self(Self::JOIN.0 | Self::INPUT.0)),
            "guest" => Some(Self::GUEST),
            "delegator" => Some(Self(Self::GUEST.0 | Self::DELEGATE.0)),
            "builder" => Some(Self(Self::GUEST.0 | Self::BUILDER.0)),
            "moderator" => Some(Self(Self::GUEST.0 | Self::MODERATOR.0)),
            _ => None,
        }
    }

    /// The names accepted by [`Self::from_role`].
    pub const ROLES: &'static [&'static str] = &[
        "spectator",
        "observer",
        "player",
        "guest",
        "delegator",
        "builder",
        "moderator",
    ];

    /// Raw scope bits, as journaled.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Scopes from raw bits; `None` if any bit names no scope. The admin
    /// authority bits round-trip here even though they are outside [`Self::ALL`].
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// Every scope either set includes.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Only the scopes both sets include.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Whether this scope set includes `required`.
    #[must_use]
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    /// This set with the scopes it includes by implication: `CHAT` reads, so
    /// it carries `HISTORY`. Apply it wherever sets are compared as sets.
    #[must_use]
    pub const fn implied(self) -> Self {
        if self.contains(Self::CHAT) {
            Self(self.0 | Self::HISTORY.0)
        } else {
            self
        }
    }

    /// Whether a participant with this set may see chat lines.
    #[must_use]
    pub const fn reads_chat(self) -> bool {
        self.0 & (Self::CHAT.0 | Self::HISTORY.0) != 0
    }

    /// The scopes a delegate of this set may hold: a subset that keeps `JOIN`.
    /// `None` unless this set includes `DELEGATE` and `requested` is a subset.
    /// A set with `CHAT` may delegate `HISTORY`, which `CHAT` includes.
    #[must_use]
    pub const fn attenuate(self, requested: Self) -> Option<Self> {
        if self.contains(Self::DELEGATE)
            && requested.contains(Self::JOIN)
            && self.implied().contains(requested)
        {
            Some(requested)
        } else {
            None
        }
    }

    /// Whether a participant with this set may cause the effect `cap`. Every
    /// connected participant may read; `chat.say` also needs `CHAT`.
    #[must_use]
    pub fn allows_effect(self, cap: &str) -> bool {
        match crate::world_caps::PROVIDER_REGISTRY
            .get(cap)
            .map_or(crate::world_caps::RequiredScope::Join, |provider| {
                provider.required_scope
            }) {
            crate::world_caps::RequiredScope::Chat => self.contains(Self::CHAT),
            // Reading chat needs only the right to read it (`CHAT` or
            // `HISTORY`), the same rule the event stream applies.
            crate::world_caps::RequiredScope::ChatRead => self.reads_chat(),
            crate::world_caps::RequiredScope::Join => self.contains(Self::JOIN),
        }
    }

    /// Scopes named on the wire (`join`, `chat`, `history`, `input`,
    /// `delegate`, `builder`, `moderator`), or `None` if a name is unknown.
    #[must_use]
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        names.into_iter().try_fold(Self(0), |scopes, name| {
            let bit = match name {
                "join" => Self::JOIN,
                "chat" => Self::CHAT,
                "history" => Self::HISTORY,
                "input" => Self::INPUT,
                "delegate" => Self::DELEGATE,
                "builder" => Self::BUILDER,
                "moderator" => Self::MODERATOR,
                _ => return None,
            };
            Some(Self(scopes.0 | bit.0))
        })
    }
}
