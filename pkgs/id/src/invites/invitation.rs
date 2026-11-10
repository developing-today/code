//! What a joiner pastes: a bare ticket, or an `id:join?` link that names the inviter.

use anyhow::{Context, Result, anyhow, ensure};
use iroh_tickets::endpoint::EndpointTicket;

const SCHEME: &str = "id:join?";

/// A pasted ticket, or a link that names the inviter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invitation {
    /// A bare ticket. It names no inviter, so no event can be checked against a key.
    Ticket(EndpointTicket),
    /// A link: the group, the inviter's account key, and the ticket.
    Link {
        /// The group's ID.
        group: [u8; 32],
        /// The inviter's account key, the trust anchor for the invite event.
        by: [u8; 32],
        /// The endpoint to dial.
        ticket: EndpointTicket,
    },
}

impl Invitation {
    /// Reads a pasted ticket or an `id:join?` link. Surrounding whitespace is ignored.
    ///
    /// # Errors
    ///
    /// Fails if the text is neither a standard endpoint ticket nor a well-formed link.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if let Some(query) = text.strip_prefix(SCHEME) {
            return parse_link(query);
        }
        let ticket = parse_ticket(text)
            .context("not an invitation: expected an endpoint ticket or an id:join link")?;
        Ok(Self::Ticket(ticket))
    }

    /// The ticket to dial.
    #[must_use]
    pub const fn ticket(&self) -> &EndpointTicket {
        match self {
            Self::Ticket(ticket) | Self::Link { ticket, .. } => ticket,
        }
    }

    /// The inviter's account key, if the invitation names one.
    #[must_use]
    pub const fn inviter(&self) -> Option<[u8; 32]> {
        match self {
            Self::Ticket(_) => None,
            Self::Link { by, .. } => Some(*by),
        }
    }
}

/// The paste form: a ticket as it is, or an `id:join?` link. A link's ticket needs no escaping,
/// since a ticket is a fixed prefix and lower-case base32.
impl std::fmt::Display for Invitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ticket(ticket) => write!(f, "{ticket}"),
            Self::Link { group, by, ticket } => write!(
                f,
                "{SCHEME}group={}&ticket={ticket}&by={}",
                hex(group),
                hex(by)
            ),
        }
    }
}

/// Lower-case hex, two characters per byte.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn parse_ticket(text: &str) -> Result<EndpointTicket> {
    text.parse::<EndpointTicket>()
        .map_err(|error| anyhow!("{error}"))
}

fn parse_link(query: &str) -> Result<Invitation> {
    let mut group = None;
    let mut by = None;
    let mut ticket = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (field, value) = pair.split_once('=').context("a link field needs =")?;
        match field {
            "group" => {
                ensure!(group.is_none(), "duplicate group field in link");
                group = Some(value);
            }
            "by" => {
                ensure!(by.is_none(), "duplicate by field in link");
                by = Some(value);
            }
            "ticket" => {
                ensure!(ticket.is_none(), "duplicate ticket field in link");
                ticket = Some(value);
            }
            _ => {}
        }
    }
    let group = parse_hex32(group.context("a link needs group=")?, "the group ID")?;
    let by = parse_hex32(
        by.context("a link needs by=, the inviter's key")?,
        "the inviter key",
    )?;
    let ticket = parse_ticket(ticket.context("a link needs ticket=")?)
        .context("the link's ticket is not an endpoint ticket")?;
    Ok(Invitation::Link { group, by, ticket })
}

/// Reads 64 hex characters as 32 bytes. Upper and lower case are both accepted.
fn parse_hex32(text: &str, what: &str) -> Result<[u8; 32]> {
    let bytes = text.as_bytes();
    ensure!(
        bytes.len() == 64 && bytes.iter().all(u8::is_ascii_hexdigit),
        "{what} must be 64 hex characters"
    );
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(out)
}
