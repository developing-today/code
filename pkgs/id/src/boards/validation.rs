use std::io::{self, Write};

use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};

use super::*;

impl BoardObject {
    pub(super) fn validate_shape(&self) -> Result<()> {
        match self {
            Self::Board(value) => validate_board_id(&value.board_id)?,
            Self::Rules(value) => {
                ensure!(value.revision > 0, "rules revision numbers start at 1");
                validate_text("rules text", &value.text, false)?;
            }
            Self::Forum(value) => validate_text("forum name", &value.name, false)?,
            Self::Thread(value) => validate_text("thread title", &value.title, false)?,
            Self::Post(value) => {
                validate_text("post body", &value.body, true)?;
                ensure!(
                    value.attachments.len() <= MAX_BOARD_REFERENCES,
                    "a post names at most {MAX_BOARD_REFERENCES} attachments"
                );
                let unique: BTreeSet<_> = value.attachments.iter().collect();
                ensure!(
                    unique.len() == value.attachments.len(),
                    "duplicate attachment reference"
                );
                if let Some(note_ref) = &value.note_ref {
                    validate_text("note home server", &note_ref.home_server, false)?;
                    ensure!(
                        is_public_key(&note_ref.sharer),
                        "note sharer is not canonical Ed25519 hex"
                    );
                }
                if let Some(passage) = value
                    .note_anchor
                    .as_ref()
                    .and_then(|anchor| anchor.passage.as_ref())
                {
                    validate_text("note passage section", &passage.section_id, false)?;
                }
            }
            Self::Attachment(value) => {
                validate_text("attachment media type", &value.media_type, false)?;
                ensure!(
                    value.media_type.len() <= 128,
                    "media type is at most 128 bytes"
                );
            }
            Self::Membership(value) => {
                ensure!(
                    is_public_key(&value.member),
                    "membership key is not canonical Ed25519 hex"
                );
                if let Some(name) = &value.display_name {
                    validate_text("display name", name, false)?;
                }
            }
            Self::RoleAttestation(value) => {
                ensure!(
                    is_public_key(&value.member),
                    "role subject is not canonical Ed25519 hex"
                );
            }
            Self::ModerationAction(value) => {
                if let Some(reason) = &value.reason {
                    validate_text("moderation reason", reason, true)?;
                }
            }
            Self::BoardFork(value) => {
                ensure!(
                    value.source_frontier.len() <= MAX_BOARD_REFERENCES,
                    "a fork cites at most {MAX_BOARD_REFERENCES} source objects"
                );
                let unique: BTreeSet<_> = value.source_frontier.iter().collect();
                ensure!(
                    unique.len() == value.source_frontier.len(),
                    "duplicate source frontier reference"
                );
            }
            Self::Theme(value) => {
                validate_text("theme name", &value.name, false)?;
                ensure!(
                    value.tokens.len() <= MAX_THEME_TOKENS,
                    "a theme names at most {MAX_THEME_TOKENS} tokens"
                );
                for (name, token) in &value.tokens {
                    validate_token_name(name)?;
                    ensure!(
                        !token.is_empty()
                            && token.len() <= MAX_THEME_TOKEN_VALUE_BYTES
                            && !token.contains('\0'),
                        "theme token value must be 1 to {MAX_THEME_TOKEN_VALUE_BYTES} bytes without NUL"
                    );
                }
                ensure!(
                    value.assets.len() <= MAX_BOARD_REFERENCES,
                    "a theme names at most {MAX_BOARD_REFERENCES} assets"
                );
                let unique: BTreeSet<_> = value.assets.iter().collect();
                ensure!(
                    unique.len() == value.assets.len(),
                    "duplicate theme asset reference"
                );
            }
            Self::GamePost(value) => {
                validate_text("game package version", &value.package.version, false)?;
                ensure!(
                    value.package.version.len() <= 128,
                    "game package version is at most 128 bytes"
                );
                validate_game_participants("game post", &value.participants)?;
                validate_game_data("game configuration", &value.configuration)?;
                validate_game_data("game genesis state", &value.genesis)?;
            }
            Self::GameChallengeAcceptance(_) => {}
            Self::GameSavePoint(value) => {
                validate_text("game package version", &value.package.version, false)?;
                ensure!(
                    value.package.version.len() <= 128,
                    "game package version is at most 128 bytes"
                );
            }
            Self::GameHandoff(value) => {
                ensure!(
                    is_public_key(&value.recipient),
                    "game handoff recipient is not a canonical Ed25519 key"
                );
                validate_game_data("game handoff terms", &value.terms)?;
            }
            Self::GameHandoffAcceptance(_) => {}
            Self::GameFork(value) => {
                validate_game_participants("game fork", &value.participants)?;
            }
            Self::GameTurn(value) => {
                validate_game_role(&value.role)?;
                validate_game_data("game turn action", &value.action)?;
            }
            Self::BoardListing(value) => {
                ensure!(
                    is_public_key(&value.host),
                    "listing host is not canonical Ed25519 hex"
                );
                ensure!(value.version > 0, "listing version numbers start at 1");
                if let Some(contact) = &value.contact {
                    validate_text("listing contact", contact, false)?;
                }
            }
            Self::Supersede(value) => ensure!(
                value.previous != value.replacement,
                "an object cannot supersede itself"
            ),
            Self::Tombstone(value) => {
                if let Some(reason) = &value.reason {
                    validate_text("tombstone reason", reason, true)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_board_id(value: &str) -> Result<()> {
    ensure!(
        (1..=128).contains(&value.len())
            && value.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"-_.".contains(&byte)),
        "board ID must be 1-128 lower-case ASCII letters, digits, '-', '_' or '.'"
    );
    Ok(())
}

fn validate_text(name: &str, value: &str, allow_empty: bool) -> Result<()> {
    ensure!(
        (allow_empty || !value.is_empty()) && value.len() <= MAX_BOARD_TEXT_BYTES,
        "{name} must be {} to {MAX_BOARD_TEXT_BYTES} bytes",
        if allow_empty { "0" } else { "1" }
    );
    ensure!(!value.contains('\0'), "{name} must not contain NUL");
    Ok(())
}

struct SerializedSizeLimit {
    limit: usize,
    written: usize,
}

impl Write for SerializedSizeLimit {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.written);
        let written = buffer.len().min(remaining);
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Lower-case hex SHA-256 of `bytes`, the canonical package-hash form.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_game_participants(name: &str, participants: &[GameParticipant]) -> Result<()> {
    ensure!(
        !participants.is_empty() && participants.len() <= MAX_GAME_PARTICIPANTS,
        "a {name} assigns 1 to {MAX_GAME_PARTICIPANTS} participants"
    );
    let mut participant_roles = BTreeSet::new();
    for participant in participants {
        ensure!(
            is_public_key(&participant.author),
            "game participant is not a canonical Ed25519 key"
        );
        validate_game_role(&participant.role)?;
        ensure!(
            participant_roles.insert((participant.author.as_str(), participant.role.as_str())),
            "duplicate game participant (author, role) assignment"
        );
    }
    Ok(())
}

fn validate_game_data(name: &str, value: &serde_json::Value) -> Result<()> {
    let mut writer = SerializedSizeLimit {
        limit: MAX_GAME_DATA_BYTES,
        written: 0,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(()),
        Err(_) if writer.written == MAX_GAME_DATA_BYTES => {
            bail!("{name} is at most {MAX_GAME_DATA_BYTES} serialized bytes")
        }
        Err(error) => Err(error.into()),
    }
}

fn validate_token_name(value: &str) -> Result<()> {
    validate_ascii_identifier("theme token name", value, 64)
}

fn validate_game_role(value: &str) -> Result<()> {
    validate_ascii_identifier("game role identifier", value, 64)
}

fn validate_ascii_identifier(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    ensure!(
        (1..=max_bytes).contains(&value.len())
            && value.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"-_.".contains(&byte)),
        "{name} must be 1-{max_bytes} lower-case ASCII letters, digits, '-', '_' or '.'"
    );
    Ok(())
}

pub(super) fn is_public_key(value: &str) -> bool {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return false;
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = (pair[0] as char).to_digit(16);
        let low = (pair[1] as char).to_digit(16);
        let (Some(high), Some(low)) = (high, low) else {
            return false;
        };
        bytes[index] = ((high << 4) | low) as u8;
    }
    VerifyingKey::from_bytes(&bytes).is_ok()
}
