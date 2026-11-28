//! Client-side capability providers (F13.7 step 5, #10930): the manifests
//! that run on the originating session's client rather than the host.
//! Split out of `world_caps.rs` to keep that file under the production-line
//! limit (#13057). Re-exported from `world_caps`, so every existing path
//! is unchanged.

use super::*;

/// The name of the first client-side provider (F13.7 step 5).
pub const CLIENT_FILE_READ_NAME: &str = "client.file.read";

/// Argument schema for `client.file.read`: one required `path` string,
/// interpreted by the client executor relative to its user-selected
/// root (never as a host path).
const CLIENT_FILE_READ_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "path",
    value_type: ArgumentType::String,
    required: true,
}];

/// Largest single read `client.file.read` may return, in bytes: the
/// design's starting bound for the first slice (32 KiB), mirrored by
/// the client shell's `LocalBounds::DESIGN_DEFAULT`.
pub const MAX_CLIENT_FILE_READ_BYTES: usize = 32_768;

/// Bounds for `client.file.read`: argument bytes plus the read cap.
const CLIENT_FILE_READ_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::ResponseBytes(MAX_CLIENT_FILE_READ_BYTES),
];

/// The `client.file.read` manifest (F13.7 step 5, design
/// `doc/2026-11-01_design_id789.md`): the first provider executed by
/// the originating session's client rather than the host. It registers
/// exactly like a production provider — one manifest entry — and is
/// excluded from every default ceiling, so registration alone widens
/// no world's authority: both the world ceiling/grant and the local
/// client grant must permit it before any read can happen.
pub const CLIENT_FILE_READ_MANIFEST: ProviderManifest = ProviderManifest {
    name: CLIENT_FILE_READ_NAME,
    version: 1,
    kind: ProviderKind::Request,
    argument_schema: ArgumentSchema::JsonObject(CLIENT_FILE_READ_ARGUMENTS),
    bounds: CLIENT_FILE_READ_BOUNDS,
    required_scope: RequiredScope::Join,
    default_ceiling: DefaultCeiling::Excluded,
    side: ProviderSide::Client,
    class: cap_class::CLASS_UNKNOWN,
};

/// The name of the client-side notification provider for the mobile
/// shell (#10930, shells v1 slice 4; design
/// `doc/2026-11-10T20-40-00Z_design_more_shells_v1` §1 and
/// §"Capability providers").
pub const CLIENT_NOTIFY_NAME: &str = "client.notify";

/// Argument schema for `client.notify`: one optional `head` — the
/// journal head sequence the re-sync hint refers to. The schema
/// deliberately carries no text or content field: a notification
/// under this provider is a hint to re-sync, never a data channel
/// (the shells record: "a notification is a hint to re-sync,
/// carrying no world content of its own"), and the shell renders
/// its own local wording. A shell already past `head` can ignore
/// the hint; a shell behind it re-syncs on return.
const CLIENT_NOTIFY_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "head",
    value_type: ArgumentType::UnsignedInteger,
    required: false,
}];

/// Largest reply `client.notify` may return, in bytes: the
/// executor's answer is a fixed-shape acknowledgment carrying no
/// content, so the bound is a small constant rather than a
/// content-sized cap.
pub const MAX_CLIENT_NOTIFY_RESPONSE_BYTES: usize = 1024;

/// Bounds for `client.notify`: argument bytes plus the
/// acknowledgment response cap.
const CLIENT_NOTIFY_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::ResponseBytes(MAX_CLIENT_NOTIFY_RESPONSE_BYTES),
];

/// The `client.notify` manifest (#10930): the mobile shell's
/// notification power, declared in the F12.4 registry shape like
/// every v1 shell power (provider declaration in v1 rides the
/// registry/manifest out of band — the `join` frame has no
/// declaration field). It registers exactly like `client.file.read`
/// — one manifest entry, client-side, excluded from every default
/// ceiling — so registration alone widens no world's authority:
/// the world ceiling/grant and the local user grant must both
/// permit it before any hint can be delivered.
pub const CLIENT_NOTIFY_MANIFEST: ProviderManifest = ProviderManifest {
    name: CLIENT_NOTIFY_NAME,
    version: 1,
    kind: ProviderKind::Request,
    argument_schema: ArgumentSchema::JsonObject(CLIENT_NOTIFY_ARGUMENTS),
    bounds: CLIENT_NOTIFY_BOUNDS,
    required_scope: RequiredScope::Join,
    default_ceiling: DefaultCeiling::Excluded,
    side: ProviderSide::Client,
    class: cap_class::CLASS_UNKNOWN,
};

/// Production client-side entries, assembled into the registry after
/// the host production entries. They are deliberately not part of the
/// macro-generated legacy [`CATALOG`] name view: that view is the host
/// catalog compatibility surface, and adding a client provider must
/// not edit it (F13.7 acceptance: registry growth without a host
/// catalog edit).
///
/// This is the declared client segment, and the source of truth the
/// registry exit proofs assert against: the assembled registry is
/// exactly the legacy [`CATALOG`] entries, then these entries, then
/// any feature-gated registry-only additions — nothing else.
pub const CLIENT_CATALOG_ENTRIES: &[ProviderManifest] =
    &[CLIENT_FILE_READ_MANIFEST, CLIENT_NOTIFY_MANIFEST];
