//! Signed artifacts: each account's own signed statements, kept as one
//! append-only chain per author.
//!
//! An artifact names its author's Ed25519 key, a per-author sequence number and
//! the hash of the artifact before it, and is signed by that key. Any server
//! can check one without trusting the server that sent it. [`ArtifactLog`]
//! holds only chains that verify, and takes neither a gap nor a fork. A kind
//! this server does not know is stored but never applied, so a newer peer
//! cannot break an older server. Applying a known kind is the directory's job.
//!
//! An artifact may also name, in `after`, the hashes of artifacts its author
//! had applied before writing it. Those names are signed, so a receiver can
//! order concurrent statements; enforcing that order is up to the consumer.
//!
//! Generic kinds carry data rather than directory changes: a `document` holds
//! structured JSON, and a `tag` labels another artifact by its hash. An author
//! tags their own document with `name` `author` and `value` their key.
//!
//! Artifacts travel over iroh only: pushed to a home server on
//! [`PUSH_ALPN`], and pulled by account from the same endpoint on [`PULL_ALPN`].

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::directory::{Directory, Refusal, hex_to_array, normalize_key};
use crate::directory_auth::Caller;
use crate::directory_view::{DirectoryAction, DirectoryOutcome};
use crate::envelope_net::{Reply, answer_frames, send};
use crate::envelope_outbox::{Outcome, check_target};
use crate::world::hex_encode;
use crate::world_hub::WorldHub;
use crate::world_net::{read_frame, write_frame};

/// The file, beside `directory.jsonl`, that holds a world's artifacts.
pub const FILE: &str = "artifacts.jsonl";
/// Artifacts in one page, which is also the most a pull applies per request.
pub const PAGE_LIMIT: usize = 100;
/// Home servers one declaration may name.
pub const MAX_HOME_SERVERS: usize = 8;
/// Hashes one artifact may name in `after`.
pub const MAX_AFTER: usize = 32;
/// The largest content a document may carry, as serialized JSON.
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
/// The kind of a friend removal.
pub const FRIEND_REMOVE: &str = "friend_remove";
/// The kind of a home-server declaration.
pub const HOME_DECLARED: &str = "home_declared";
/// The kind of a generic structured document.
pub const DOCUMENT: &str = "document";
/// The kind of a label on another artifact.
pub const TAG: &str = "tag";

/// ALPN on which a server accepts pushed artifacts, one per stream.
pub const PUSH_ALPN: &[u8] = b"/id-artifact-push/1";
/// ALPN on which a server answers pulls of an account's artifacts.
pub const PULL_ALPN: &[u8] = b"/id-artifact-pull/1";

/// One signed statement by an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedArtifact {
    /// Hex Ed25519 public key of the account that signed it.
    pub author: String,
    /// One-based position in the author's chain.
    pub seq: u64,
    /// Hash of the author's artifact at `seq - 1`, empty for `seq` 1.
    pub prev: String,
    /// What the body says. An unknown kind is kept but not applied.
    pub kind: String,
    /// The kind-specific fields.
    pub body: serde_json::Value,
    /// Hashes of artifacts the author had applied before this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    /// Hex SHA-256 over the signed body.
    pub hash: String,
    /// Hex Ed25519 signature over the signed body.
    pub signature: String,
}

#[derive(Serialize)]
struct SignedBody<'a> {
    author: &'a str,
    seq: u64,
    prev: &'a str,
    kind: &'a str,
    body: &'a serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    after: Vec<String>,
}

fn signed_bytes(
    author: &str,
    seq: u64,
    prev: &str,
    kind: &str,
    body: &serde_json::Value,
    after: &[String],
) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&SignedBody {
        author,
        seq,
        prev,
        kind,
        body,
        after: after.to_vec(),
    })?)
}

fn is_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A friendship ends, and the request IDs it covers are void.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRemove {
    /// One account.
    pub a: String,
    /// The other account.
    pub b: String,
    /// Request IDs the removal covers: the one that formed the friendship, and any still pending.
    #[serde(default)]
    pub requests: Vec<String>,
}

/// Where an account's artifacts are held: an endpoint ticket or a bare node ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeServer {
    /// The Iroh endpoint that takes pushes and serves pulls of the account's artifacts.
    pub endpoint: String,
    /// The world the server holds the account in.
    pub world_id: String,
}

/// The servers an account publishes its artifacts to. A newer declaration replaces an older one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeDeclared {
    /// The declaring account.
    pub account: String,
    /// Where its artifacts are held.
    pub servers: Vec<HomeServer>,
}

/// Structured data an account publishes as its own statement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// What the content is, such as `application/json`.
    pub media_type: String,
    /// The content itself.
    pub content: serde_json::Value,
}

/// A label on another artifact, which the subject names by its hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// The hash of the artifact labelled.
    pub subject: String,
    /// The label, such as `author`.
    pub name: String,
    /// The label's value, such as an account key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A known kind, with its body decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A friendship ends.
    FriendRemove(FriendRemove),
    /// An account names its home servers.
    HomeDeclared(HomeDeclared),
    /// An account publishes structured data.
    Document(Document),
    /// An account labels an artifact.
    Tag(Tag),
}

impl Kind {
    fn encode(&self) -> Result<(&'static str, serde_json::Value)> {
        Ok(match self {
            Self::FriendRemove(body) => (FRIEND_REMOVE, serde_json::to_value(body)?),
            Self::HomeDeclared(body) => (HOME_DECLARED, serde_json::to_value(body)?),
            Self::Document(body) => (DOCUMENT, serde_json::to_value(body)?),
            Self::Tag(body) => (TAG, serde_json::to_value(body)?),
        })
    }
}

fn forbidden(message: impl Into<String>) -> anyhow::Error {
    Refusal::Forbidden(message.into()).into()
}

fn check_key(key: &str) -> Result<()> {
    match normalize_key(key) {
        Ok(canonical) if canonical == key => Ok(()),
        _ => Err(forbidden(format!("{key} is not an account key"))),
    }
}

impl SignedArtifact {
    /// Sign a body of `kind` as the author holding `key`, at `seq` after `prev`.
    ///
    /// # Errors
    ///
    /// Fails if `seq` is zero or the body cannot be serialized.
    pub fn sign(
        key: &SigningKey,
        seq: u64,
        prev: &str,
        kind: &str,
        body: serde_json::Value,
    ) -> Result<Self> {
        Self::sign_after(key, seq, prev, kind, body, Vec::new())
    }

    /// Like [`Self::sign`], also naming `after`, the artifacts the author had applied before this one.
    ///
    /// # Errors
    ///
    /// Fails if `seq` is zero or the body cannot be serialized.
    pub fn sign_after(
        key: &SigningKey,
        seq: u64,
        prev: &str,
        kind: &str,
        body: serde_json::Value,
        after: Vec<String>,
    ) -> Result<Self> {
        ensure!(seq >= 1, "an artifact's sequence number starts at 1");
        let author = hex_encode(&key.verifying_key().to_bytes());
        let bytes = signed_bytes(&author, seq, prev, kind, &body, &after)?;
        Ok(Self {
            hash: hex_encode(&Sha256::digest(&bytes)),
            signature: hex_encode(&key.sign(&bytes).to_bytes()),
            author,
            seq,
            prev: prev.to_owned(),
            kind: kind.to_owned(),
            body,
            after,
        })
    }

    /// Check the hash against the content and the signature against the author key.
    ///
    /// # Errors
    ///
    /// Fails with [`Refusal::Unauthenticated`] if either does not check out.
    pub fn verify(&self) -> Result<()> {
        self.check()
            .map_err(|error| Refusal::Unauthenticated(format!("{error:#}")).into())
    }

    fn check(&self) -> Result<()> {
        ensure!(
            normalize_key(&self.author)? == self.author,
            "author is not a canonical key"
        );
        ensure!(
            self.after.len() <= MAX_AFTER,
            "an artifact names at most {MAX_AFTER} artifacts before it"
        );
        ensure!(
            self.after.iter().all(|hash| is_hash(hash)),
            "an artifact names a malformed hash in after"
        );
        let bytes = signed_bytes(
            &self.author,
            self.seq,
            &self.prev,
            &self.kind,
            &self.body,
            &self.after,
        )?;
        ensure!(
            hex_encode(&Sha256::digest(&bytes)) == self.hash,
            "artifact hash does not match its content"
        );
        let key = VerifyingKey::from_bytes(
            &hex_to_array::<32>(&self.author).context("malformed author key")?,
        )
        .context("author is not an Ed25519 key")?;
        let signature = Signature::from_bytes(
            &hex_to_array::<64>(&self.signature).context("malformed artifact signature")?,
        );
        key.verify(&bytes, &signature)
            .context("signature does not verify against the author key")
    }

    /// The known kind this artifact carries, checked. `None` for a kind this
    /// server does not know: it is stored but never applied.
    ///
    /// # Errors
    ///
    /// Fails with [`Refusal::Forbidden`] when a known kind is malformed or its
    /// author may not make it.
    pub fn parsed(&self) -> Result<Option<Kind>> {
        match self.kind.as_str() {
            FRIEND_REMOVE => {
                let body: FriendRemove = self.decode()?;
                check_key(&body.a)?;
                check_key(&body.b)?;
                if body.a == body.b {
                    return Err(forbidden("no one befriends themselves"));
                }
                if self.author != body.a && self.author != body.b {
                    return Err(forbidden(
                        "a friend removal must be signed by one of its two accounts",
                    ));
                }
                Ok(Some(Kind::FriendRemove(body)))
            }
            HOME_DECLARED => {
                let body: HomeDeclared = self.decode()?;
                check_key(&body.account)?;
                if self.author != body.account {
                    return Err(forbidden(
                        "a home declaration must be signed by its own account",
                    ));
                }
                if body.servers.len() > MAX_HOME_SERVERS {
                    return Err(forbidden(format!(
                        "a declaration names at most {MAX_HOME_SERVERS} servers"
                    )));
                }
                for server in &body.servers {
                    check_target(&server.endpoint)
                        .map_err(|error| forbidden(format!("{error:#}")))?;
                    if server.world_id.is_empty() {
                        return Err(forbidden("a home server needs a world ID"));
                    }
                }
                Ok(Some(Kind::HomeDeclared(body)))
            }
            DOCUMENT => {
                let body: Document = self.decode()?;
                if body.media_type.is_empty() || body.media_type.len() > 128 {
                    return Err(forbidden("a document names its media type"));
                }
                let size = serde_json::to_vec(&body.content)
                    .map_err(|error| forbidden(format!("{error}")))?
                    .len();
                if size > MAX_DOCUMENT_BYTES {
                    return Err(forbidden(format!(
                        "a document holds at most {MAX_DOCUMENT_BYTES} bytes of content"
                    )));
                }
                Ok(Some(Kind::Document(body)))
            }
            TAG => {
                let body: Tag = self.decode()?;
                if !is_hash(&body.subject) {
                    return Err(forbidden("a tag names its subject by hash"));
                }
                if body.name.is_empty() || body.name.len() > 64 {
                    return Err(forbidden("a tag's name is 1 to 64 bytes"));
                }
                if body.value.as_ref().is_some_and(|value| value.len() > 256) {
                    return Err(forbidden("a tag's value is at most 256 bytes"));
                }
                Ok(Some(Kind::Tag(body)))
            }
            _ => Ok(None),
        }
    }

    fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.body.clone())
            .map_err(|error| forbidden(format!("{} body is malformed: {error}", self.kind)))
    }
}

/// Whether an insert stored a new artifact or matched one already held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stored {
    /// The artifact extends its author's chain.
    New,
    /// The same artifact is already held.
    Duplicate,
}

/// Every author's chain of verified artifacts, in sequence order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtifactLog {
    chains: BTreeMap<String, Vec<SignedArtifact>>,
}

impl ArtifactLog {
    /// Add an artifact to its author's chain, if it verifies and extends the chain.
    ///
    /// # Errors
    ///
    /// Fails with [`Refusal::Unauthenticated`] for a bad signature or hash, and
    /// with [`Refusal::Conflict`] for a gap, a fork, or a `prev` that does not
    /// match. The log is unchanged on error.
    pub fn insert(&mut self, artifact: SignedArtifact) -> Result<Stored> {
        artifact.verify()?;
        artifact.parsed()?;
        if artifact.seq < 1 {
            return Err(forbidden("sequence numbers start at 1"));
        }
        let held = self
            .chains
            .get(&artifact.author)
            .map_or(&[][..], Vec::as_slice);
        let have = u64::try_from(held.len())?;
        if artifact.seq <= have {
            let same = held
                .get(usize::try_from(artifact.seq - 1)?)
                .is_some_and(|at| at.hash == artifact.hash);
            return if same {
                Ok(Stored::Duplicate)
            } else {
                Err(Refusal::Conflict(format!(
                    "artifact {} of this account is already held as a different one",
                    artifact.seq
                ))
                .into())
            };
        }
        if artifact.seq > have + 1 {
            return Err(Refusal::Conflict(format!(
                "artifact {} is missing; it must be pulled before artifact {}",
                have + 1,
                artifact.seq
            ))
            .into());
        }
        let expected = held
            .last()
            .map_or_else(String::new, |last| last.hash.clone());
        if artifact.prev != expected {
            return Err(Refusal::Conflict(format!(
                "artifact {} does not follow the one held before it",
                artifact.seq
            ))
            .into());
        }
        self.chains
            .entry(artifact.author.clone())
            .or_default()
            .push(artifact);
        Ok(Stored::New)
    }

    /// Every held artifact, author by author, each in sequence order.
    pub fn all(&self) -> impl Iterator<Item = &SignedArtifact> {
        self.chains.values().flatten()
    }

    /// The number of artifacts held for an author, which is the last sequence number.
    #[must_use]
    pub fn head_seq(&self, author: &str) -> u64 {
        self.chains
            .get(author)
            .map_or(0, |chain| u64::try_from(chain.len()).unwrap_or(u64::MAX))
    }

    /// Up to [`PAGE_LIMIT`] of an author's artifacts with a sequence number after `after`, in order.
    #[must_use]
    pub fn page(&self, author: &str, after: u64) -> Vec<SignedArtifact> {
        self.chains
            .get(author)
            .into_iter()
            .flatten()
            .filter(|artifact| artifact.seq > after)
            .take(PAGE_LIMIT)
            .cloned()
            .collect()
    }

    /// The latest home-server declaration an account has made.
    #[must_use]
    pub fn declared(&self, account: &str) -> Option<HomeDeclared> {
        self.chains
            .get(account)?
            .iter()
            .rev()
            .find_map(|artifact| match artifact.parsed() {
                Ok(Some(Kind::HomeDeclared(declared))) => Some(declared),
                _ => None,
            })
    }

    /// Sign `kind` as the author holding `key`, at the position after its chain's head.
    /// The result is not added to the log.
    ///
    /// # Errors
    ///
    /// Fails if the body cannot be serialized.
    pub fn sign(&self, key: &SigningKey, kind: &Kind) -> Result<SignedArtifact> {
        self.sign_after(key, kind, Vec::new())
    }

    /// Like [`Self::sign`], also naming `after`, the artifacts the author had applied before this one.
    ///
    /// # Errors
    ///
    /// Fails if the body cannot be serialized.
    pub fn sign_after(
        &self,
        key: &SigningKey,
        kind: &Kind,
        after: Vec<String>,
    ) -> Result<SignedArtifact> {
        let author = hex_encode(&key.verifying_key().to_bytes());
        let (seq, prev) = self.chains.get(&author).map_or_else(
            || (1, String::new()),
            |chain| {
                let last = chain.last();
                (
                    last.map_or(1, |last| last.seq + 1),
                    last.map_or_else(String::new, |last| last.hash.clone()),
                )
            },
        );
        let (name, body) = kind.encode()?;
        SignedArtifact::sign_after(key, seq, &prev, name, body, after)
    }

    /// Load the log at `path`, which is empty if the file does not exist. A
    /// final line cut short by a crash was never synced, so it is dropped and
    /// the file is truncated to the last complete line.
    ///
    /// # Errors
    ///
    /// Fails if a complete line is corrupt or does not verify.
    pub fn open(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error).context("reading the artifact file"),
        };
        let kept = text.rfind('\n').map_or(0, |index| index + 1);
        if kept < text.len() {
            OpenOptions::new()
                .write(true)
                .open(path)?
                .set_len(u64::try_from(kept)?)?;
        }
        let mut log = Self::default();
        for (number, line) in text[..kept].lines().enumerate() {
            let artifact: SignedArtifact = serde_json::from_str(line)
                .with_context(|| format!("artifact line {} is corrupt", number + 1))?;
            log.insert(artifact)
                .with_context(|| format!("artifact line {} is refused", number + 1))?;
        }
        Ok(log)
    }
}

/// Durably append one artifact to the file at `path`.
///
/// # Errors
///
/// Fails if the file cannot be written or synced.
pub fn append(path: &Path, artifact: &SignedArtifact) -> Result<()> {
    let mut line = serde_json::to_vec(artifact)?;
    line.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

/// A page of an account's artifacts, the answer to one [`PullRequest`].
#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    /// The account's artifacts after the sequence number asked for, in order.
    pub artifacts: Vec<SignedArtifact>,
}

/// What applying a page did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PageOutcome {
    /// Artifacts newly stored.
    pub stored: usize,
    /// Artifacts already held.
    pub duplicates: usize,
    /// The first artifact refused. Nothing after it is applied, since later
    /// artifacts in a chain depend on it.
    pub refused: Option<PageRefusal>,
}

/// The artifact a page stopped at, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct PageRefusal {
    /// The refused artifact's sequence number.
    pub seq: u64,
    /// Why it was refused.
    pub reason: String,
}

/// Apply a page of `account`'s artifacts to `directory`. Each artifact is
/// checked on its own, so the server that served the page is not trusted.
pub fn apply_page(
    directory: &mut Directory,
    account: &str,
    page: &[SignedArtifact],
) -> PageOutcome {
    let mut outcome = PageOutcome::default();
    for artifact in page {
        let refusal = if artifact.author == account {
            match directory.receive_artifact(artifact) {
                Ok(Stored::New) => {
                    outcome.stored += 1;
                    None
                }
                Ok(Stored::Duplicate) => {
                    outcome.duplicates += 1;
                    None
                }
                Err(error) => Some(format!("{error:#}")),
            }
        } else {
            Some("artifact is not from the requested account".to_owned())
        };
        if let Some(reason) = refusal {
            outcome.refused = Some(PageRefusal {
                seq: artifact.seq,
                reason,
            });
            break;
        }
    }
    outcome
}

/// One pull on [`PULL_ALPN`]: the artifacts of `account` after sequence `after`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PullRequest {
    /// The account whose chain is wanted.
    pub account: String,
    /// The last sequence number the puller already holds.
    pub after: u64,
}

/// Answers [`PULL_ALPN`] pulls from the worlds a hub holds.
#[derive(Clone, Debug)]
pub struct ArtifactPullProtocol {
    hub: WorldHub,
}

impl ArtifactPullProtocol {
    /// Serve the artifacts of the worlds in `hub` to Iroh peers.
    #[must_use]
    pub const fn new(hub: WorldHub) -> Self {
        Self { hub }
    }

    async fn page(&self, request: &PullRequest) -> Result<Page> {
        check_key(&request.account)?;
        let Ok(lease) = self.hub.lease(None, None).await else {
            bail!("the server is busy; try again");
        };
        let action = DirectoryAction::ReadArtifacts {
            account: request.account.clone(),
            after: request.after,
        };
        let outcome = lease.service().directory(Caller::default(), action).await?;
        Ok(Page {
            artifacts: outcome.artifact_page.unwrap_or_default(),
        })
    }
}

fn refused(error: anyhow::Error) -> AcceptError {
    AcceptError::from_boxed(error.into())
}

impl ProtocolHandler for ArtifactPullProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let Some(frame) = read_frame(&mut recv).await.map_err(refused)? else {
            return Ok(());
        };
        let request: PullRequest =
            serde_json::from_slice(&frame).map_err(|error| refused(error.into()))?;
        let page = self.page(&request).await.map_err(refused)?;
        let bytes = serde_json::to_vec(&page).map_err(|error| refused(error.into()))?;
        write_frame(&mut send, &bytes).await.map_err(refused)?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }
}

/// Accepts pushed artifacts on [`PUSH_ALPN`] and stores them in the worlds a hub holds.
#[derive(Clone, Debug)]
pub struct ArtifactPushProtocol {
    hub: WorldHub,
}

impl ArtifactPushProtocol {
    /// Take pushed artifacts for the worlds in `hub`.
    #[must_use]
    pub const fn new(hub: WorldHub) -> Self {
        Self { hub }
    }
}

impl ProtocolHandler for ArtifactPushProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let hub = self.hub.clone();
        answer_frames(&conn, move |body| {
            let hub = hub.clone();
            async move { receive_pushed(&hub, &body).await }
        })
        .await
    }
}

/// Push `artifact` to the home server at `target`, and say whether it is done.
pub async fn push(endpoint: &Endpoint, target: EndpointAddr, artifact: &SignedArtifact) -> Outcome {
    match serde_json::to_vec(artifact) {
        Ok(json) => send(endpoint, PUSH_ALPN, target, &json).await,
        Err(error) => Outcome::Refused(format!("artifact does not encode: {error}")),
    }
}

async fn receive_pushed(hub: &WorldHub, body: &[u8]) -> Reply {
    let artifact: SignedArtifact = match serde_json::from_slice(body) {
        Ok(artifact) => artifact,
        Err(error) => {
            return Reply::Refused {
                message: format!("not an artifact: {error}"),
            };
        }
    };
    let Ok(lease) = hub.lease(None, None).await else {
        return Reply::Retry {
            message: "the server is busy; try again".to_owned(),
        };
    };
    let action = DirectoryAction::ReceiveArtifact { artifact };
    match lease.service().directory(Caller::default(), action).await {
        Ok(DirectoryOutcome {
            stored: Some(Stored::New),
            ..
        }) => Reply::Applied,
        Ok(DirectoryOutcome {
            stored: Some(Stored::Duplicate),
            ..
        }) => Reply::Duplicate,
        Ok(_) => Reply::Retry {
            message: "artifact was not stored".to_owned(),
        },
        Err(error) if error.downcast_ref::<Refusal>().is_some() => Reply::Refused {
            message: format!("{error:#}"),
        },
        Err(error) => Reply::Retry {
            message: format!("{error:#}"),
        },
    }
}

/// Fetch one page of `account`'s artifacts after `after` from the home server at `peer`.
///
/// # Errors
///
/// Fails if the peer is unreachable, does not speak [`PULL_ALPN`], or answers with
/// something other than a page.
pub async fn fetch_page(
    endpoint: &Endpoint,
    peer: EndpointAddr,
    account: &str,
    after: u64,
) -> Result<Vec<SignedArtifact>> {
    check_key(account)?;
    let connection = endpoint
        .connect(peer, PULL_ALPN)
        .await
        .context("connecting to the home server")?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .context("opening a pull stream")?;
    let request = PullRequest {
        account: account.to_owned(),
        after,
    };
    write_frame(&mut send, &serde_json::to_vec(&request)?).await?;
    send.finish().context("finishing the pull request")?;
    let frame = read_frame(&mut recv)
        .await?
        .context("home server closed without a page")?;
    let page: Page = serde_json::from_slice(&frame).context("artifact page is malformed")?;
    ensure!(
        page.artifacts.len() <= PAGE_LIMIT,
        "artifact page has too many artifacts"
    );
    connection.close(0_u32.into(), b"pulled");
    Ok(page.artifacts)
}

/// Pull `account`'s new artifacts from the home server at `peer`, page by page,
/// applying each page to `directory` as it arrives. Stops at the first refusal,
/// a short page, or a page that stores nothing.
///
/// # Errors
///
/// Fails if a page cannot be fetched.
pub async fn pull(
    endpoint: &Endpoint,
    peer: EndpointAddr,
    account: &str,
    directory: &mut Directory,
) -> Result<PageOutcome> {
    check_key(account)?;
    let mut total = PageOutcome::default();
    loop {
        let after = directory.artifacts().head_seq(account);
        let page = fetch_page(endpoint, peer.clone(), account, after).await?;
        let full = page.len() == PAGE_LIMIT;
        let outcome = apply_page(directory, account, &page);
        total.stored += outcome.stored;
        total.duplicates += outcome.duplicates;
        if outcome.refused.is_some() {
            total.refused = outcome.refused;
            break;
        }
        if !full || outcome.stored == 0 {
            break;
        }
    }
    Ok(total)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh::{
        TransportAddr,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };
    use iroh_base::SecretKey;
    use serde_json::json;
    use std::net::{Ipv4Addr, SocketAddr};
    use tempfile::TempDir;

    use crate::world::{WorldCore, WorldHandle, WorldLimits};
    use crate::world_session::WorldService;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    fn removal(a: u8, b: u8, requests: &[&str]) -> Kind {
        Kind::FriendRemove(FriendRemove {
            a: public(a),
            b: public(b),
            requests: requests.iter().map(|r| (*r).to_owned()).collect(),
        })
    }

    fn endpoint_of(seed: u8) -> String {
        SecretKey::from_bytes(&[seed; 32]).public().to_string()
    }

    fn declared(seed: u8, home: u8) -> Kind {
        Kind::HomeDeclared(HomeDeclared {
            account: public(seed),
            servers: vec![HomeServer {
                endpoint: endpoint_of(home),
                world_id: "home".to_owned(),
            }],
        })
    }

    fn document(text: &str) -> Kind {
        Kind::Document(Document {
            media_type: "application/json".to_owned(),
            content: json!({ "text": text }),
        })
    }

    fn author_tag(subject: &str, author: u8) -> Kind {
        Kind::Tag(Tag {
            subject: subject.to_owned(),
            name: "author".to_owned(),
            value: Some(public(author)),
        })
    }

    #[test]
    fn a_replayed_artifact_is_a_duplicate_and_a_gap_names_what_is_missing() {
        let mut log = ArtifactLog::default();
        let first = log.sign(&key(1), &removal(1, 2, &["r1"])).unwrap();
        assert_eq!(first.seq, 1);
        assert_eq!(log.insert(first.clone()).unwrap(), Stored::New);
        assert_eq!(log.insert(first).unwrap(), Stored::Duplicate);
        assert_eq!(log.head_seq(&public(1)), 1);

        let second = log.sign(&key(1), &removal(1, 3, &[])).unwrap();
        let third = SignedArtifact::sign(
            &key(1),
            3,
            &second.hash,
            FRIEND_REMOVE,
            json!({"a": public(1), "b": public(3), "requests": []}),
        )
        .unwrap();
        let gap = log.insert(third).unwrap_err();
        assert!(
            matches!(gap.downcast_ref::<Refusal>(), Some(Refusal::Conflict(m)) if m.contains("artifact 2 is missing")),
            "{gap:#}"
        );
        assert_eq!(log.head_seq(&public(1)), 1);
    }

    #[test]
    fn a_fork_at_a_held_sequence_number_is_refused() {
        let mut log = ArtifactLog::default();
        let held = log.sign(&key(1), &removal(1, 2, &["r1"])).unwrap();
        log.insert(held).unwrap();
        let other = SignedArtifact::sign(
            &key(1),
            1,
            "",
            FRIEND_REMOVE,
            json!({"a": public(1), "b": public(2), "requests": ["r2"]}),
        )
        .unwrap();
        let fork = log.insert(other).unwrap_err();
        assert!(
            matches!(fork.downcast_ref::<Refusal>(), Some(Refusal::Conflict(_))),
            "{fork:#}"
        );
    }

    #[test]
    fn a_forged_or_tampered_artifact_is_refused_and_changes_nothing() {
        let mut log = ArtifactLog::default();
        let good = log.sign(&key(1), &removal(1, 2, &["r1"])).unwrap();
        log.insert(good.clone()).unwrap();
        let before = log.clone();

        let mut tampered = SignedArtifact::sign(
            &key(1),
            2,
            &good.hash,
            FRIEND_REMOVE,
            json!({"a": public(1), "b": public(2), "requests": []}),
        )
        .unwrap();
        tampered.body = json!({"a": public(1), "b": public(2), "requests": ["r9"]});
        let error = log.insert(tampered).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Refusal>(),
                Some(Refusal::Unauthenticated(_))
            ),
            "{error:#}"
        );

        let mut forged = SignedArtifact::sign(
            &key(3),
            2,
            &good.hash,
            FRIEND_REMOVE,
            json!({"a": public(1), "b": public(2), "requests": []}),
        )
        .unwrap();
        forged.author = public(1);
        let error = log.insert(forged).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Refusal>(),
                Some(Refusal::Unauthenticated(_))
            ),
            "{error:#}"
        );
        assert_eq!(log, before);
    }

    #[test]
    fn a_removal_must_be_signed_by_one_of_its_accounts() {
        let mut log = ArtifactLog::default();
        let stranger = SignedArtifact::sign(
            &key(3),
            1,
            "",
            FRIEND_REMOVE,
            json!({"a": public(1), "b": public(2), "requests": []}),
        )
        .unwrap();
        let error = log.insert(stranger).unwrap_err();
        assert!(
            matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Forbidden(_))),
            "{error:#}"
        );
    }

    #[test]
    fn an_unknown_kind_is_stored_and_has_no_meaning_here() {
        let mut log = ArtifactLog::default();
        let future =
            SignedArtifact::sign(&key(1), 1, "", "future_kind", json!({"anything": [1, 2]}))
                .unwrap();
        assert_eq!(log.insert(future.clone()).unwrap(), Stored::New);
        assert_eq!(future.parsed().unwrap(), None);
        assert_eq!(log.head_seq(&public(1)), 1);
    }

    #[test]
    fn a_home_declaration_is_checked_and_the_latest_one_wins() {
        let mut log = ArtifactLog::default();
        let first = log.sign(&key(1), &declared(1, 7)).unwrap();
        log.insert(first.clone()).unwrap();
        let second = log.sign(&key(1), &declared(1, 8)).unwrap();
        log.insert(second).unwrap();
        assert_eq!(
            log.declared(&public(1)).unwrap().servers[0].endpoint,
            endpoint_of(8)
        );
        assert_eq!(log.insert(first).unwrap(), Stored::Duplicate);
        assert_eq!(
            log.declared(&public(1)).unwrap().servers[0].endpoint,
            endpoint_of(8)
        );

        let plain = SignedArtifact::sign(
            &key(1),
            3,
            &log.page(&public(1), 0)[1].hash,
            HOME_DECLARED,
            json!({"account": public(1), "servers": [{"endpoint": "not a node", "world_id": "w"}]}),
        )
        .unwrap();
        let refused = log.insert(plain).unwrap_err();
        assert!(
            matches!(
                refused.downcast_ref::<Refusal>(),
                Some(Refusal::Forbidden(_))
            ),
            "{refused:#}"
        );
        let other_account = SignedArtifact::sign(
            &key(2),
            1,
            "",
            HOME_DECLARED,
            json!({"account": public(1), "servers": []}),
        )
        .unwrap();
        assert!(log.insert(other_account).is_err());
    }

    #[test]
    fn artifacts_survive_a_restart_and_a_torn_tail_is_dropped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE);
        let mut log = ArtifactLog::default();
        let first = log.sign(&key(1), &removal(1, 2, &["r1"])).unwrap();
        log.insert(first.clone()).unwrap();
        append(&path, &first).unwrap();
        let second = log.sign(&key(1), &declared(1, 7)).unwrap();
        log.insert(second.clone()).unwrap();
        append(&path, &second).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{\"author\":\"tor");
        fs::write(&path, text).unwrap();

        let reopened = ArtifactLog::open(&path).unwrap();
        assert_eq!(reopened, log);
        assert!(fs::read_to_string(&path).unwrap().ends_with('\n'));
        assert_eq!(reopened.head_seq(&public(1)), 2);
    }

    #[test]
    fn a_page_is_in_order_and_bounded() {
        let mut log = ArtifactLog::default();
        for _ in 0..(PAGE_LIMIT + 5) {
            let next = log.sign(&key(1), &removal(1, 2, &[])).unwrap();
            log.insert(next).unwrap();
        }
        let first = log.page(&public(1), 0);
        assert_eq!(first.len(), PAGE_LIMIT);
        assert_eq!(first[0].seq, 1);
        let rest = log.page(&public(1), PAGE_LIMIT as u64);
        assert_eq!(
            rest.iter().map(|a| a.seq).collect::<Vec<_>>(),
            vec![101, 102, 103, 104, 105]
        );
    }

    fn account(directory: &mut Directory, name: &str) -> (String, String) {
        let (credential, _) = directory.sign_up(name).unwrap();
        let id = directory.account_for(&credential).unwrap();
        (id, credential)
    }

    fn key_of(credential: &str) -> SigningKey {
        let secret = hex_to_array::<32>(credential.rsplit('.').next().unwrap()).unwrap();
        SigningKey::from_bytes(&secret)
    }

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    fn loopback(endpoint: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            endpoint.id(),
            endpoint.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                TransportAddr::Ip(SocketAddr::new(ip, a.port()))
            }),
        )
    }

    #[test]
    fn a_page_applies_what_verifies_and_stops_at_a_forgery() {
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
        let removal = home.remove_friend(&ann_credential, &bo).unwrap();

        let mut forged = SignedArtifact::sign(
            &key_of(&ann_credential),
            2,
            &removal.hash,
            HOME_DECLARED,
            json!({"account": ann, "servers": []}),
        )
        .unwrap();
        forged.body =
            json!({"account": ann, "servers": [{"endpoint": endpoint_of(9), "world_id": "x"}]});
        let outcome = apply_page(&mut away, &ann, &[removal, forged]);
        assert_eq!(outcome.stored, 1);
        assert_eq!(outcome.refused.map(|refused| refused.seq), Some(2));
        assert!(!away.are_friends(&ann, &bo), "the verified removal applied");
        assert_eq!(away.artifacts().head_seq(&ann), 1);
    }

    #[tokio::test]
    async fn a_pull_over_iroh_applies_the_removal_a_home_server_holds() {
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
        let removal = home.remove_friend(&ann_credential, &bo).unwrap();

        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, None);
        let stored = service
            .directory(
                Caller::default(),
                DirectoryAction::ReceiveArtifact { artifact: removal },
            )
            .await
            .unwrap();
        assert_eq!(stored.stored, Some(Stored::New));

        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(
                PULL_ALPN,
                ArtifactPullProtocol::new(WorldHub::single(service)),
            )
            .spawn();
        let client = endpoint().await;
        let outcome = pull(&client, loopback(&server), &ann, &mut away)
            .await
            .unwrap();
        assert_eq!(outcome.stored, 1);
        assert!(outcome.refused.is_none());
        assert!(!away.are_friends(&ann, &bo));
        assert_eq!(away.artifacts().head_seq(&ann), 1);
    }

    #[test]
    fn a_pulled_artifact_from_another_account_is_refused() {
        let mut directory = Directory::new();
        let stranger = SignedArtifact::sign(
            &key(5),
            1,
            "",
            HOME_DECLARED,
            json!({"account": public(5), "servers": []}),
        )
        .unwrap();
        let outcome = apply_page(&mut directory, &public(1), &[stranger]);
        assert_eq!(outcome.stored, 0);
        assert!(outcome.refused.is_some());
    }

    #[test]
    fn causal_references_are_signed_and_bounded() {
        let mut log = ArtifactLog::default();
        let first = log.sign(&key(1), &document("first")).unwrap();
        log.insert(first.clone()).unwrap();
        let second = log
            .sign_after(&key(1), &document("second"), vec![first.hash.clone()])
            .unwrap();
        assert_eq!(second.after, vec![first.hash.clone()]);
        log.insert(second.clone()).unwrap();

        let mut stripped = second;
        stripped.after.clear();
        let error = ArtifactLog::default().insert(stripped).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Refusal>(),
                Some(Refusal::Unauthenticated(_))
            ),
            "{error:#}"
        );

        let malformed = log
            .sign_after(&key(1), &document("third"), vec!["not a hash".to_owned()])
            .unwrap();
        assert!(log.insert(malformed).is_err());
        let crowded = log
            .sign_after(
                &key(1),
                &document("fourth"),
                vec![first.hash; MAX_AFTER + 1],
            )
            .unwrap();
        assert!(log.insert(crowded).is_err());
        assert_eq!(log.head_seq(&public(1)), 2);
    }

    #[test]
    fn documents_and_tags_are_checked_and_never_applied_as_directory_changes() {
        let huge = Kind::Document(Document {
            media_type: "application/json".to_owned(),
            content: json!({ "text": "x".repeat(MAX_DOCUMENT_BYTES) }),
        });
        assert!(
            ArtifactLog::default()
                .sign(&key(1), &huge)
                .unwrap()
                .parsed()
                .is_err()
        );

        let loose = Kind::Tag(Tag {
            subject: "not a hash".to_owned(),
            name: "author".to_owned(),
            value: None,
        });
        assert!(
            ArtifactLog::default()
                .sign(&key(1), &loose)
                .unwrap()
                .parsed()
                .is_err()
        );

        let fine = ArtifactLog::default()
            .sign(&key(1), &document("ok"))
            .unwrap();
        assert_eq!(fine.parsed().unwrap(), Some(document("ok")));
        let tag = ArtifactLog::default()
            .sign(&key(1), &author_tag(&fine.hash, 1))
            .unwrap();
        assert!(matches!(tag.parsed(), Ok(Some(Kind::Tag(_)))));

        let mut directory = Directory::new();
        let stored = directory
            .receive_artifact(
                &ArtifactLog::default()
                    .sign(&key(1), &document("x"))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(stored, Stored::New);
        assert_eq!(directory.artifacts().head_seq(&public(1)), 1);
    }

    #[tokio::test]
    async fn a_document_and_its_author_tag_travel_over_iroh() {
        let mut author = Directory::new();
        let (ann, credential) = account(&mut author, "Ann");
        let document = author
            .publish_document(
                &credential,
                "application/json",
                json!({ "title": "Notes", "items": [1, 2] }),
                Vec::new(),
            )
            .unwrap();
        let tag = author
            .publish_tag(
                &credential,
                &document.hash,
                "author",
                Some(ann.clone()),
                Vec::new(),
            )
            .unwrap();
        assert_eq!(tag.seq, 2);

        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let hub = WorldHub::single(WorldService::new(world, None));
        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(PUSH_ALPN, ArtifactPushProtocol::new(hub.clone()))
            .accept(PULL_ALPN, ArtifactPullProtocol::new(hub))
            .spawn();
        let client = endpoint().await;
        let home = loopback(&server);

        assert_eq!(
            push(&client, home.clone(), &document).await,
            Outcome::Delivered
        );
        assert_eq!(push(&client, home.clone(), &tag).await, Outcome::Delivered);
        assert_eq!(push(&client, home.clone(), &tag).await, Outcome::Delivered);

        let mut reader = Directory::new();
        let outcome = pull(&client, home, &ann, &mut reader).await.unwrap();
        assert_eq!(outcome.stored, 2);
        let pulled = reader.artifacts().page(&ann, 0);
        assert_eq!(pulled, vec![document.clone(), tag.clone()]);
        match tag.parsed().unwrap() {
            Some(Kind::Tag(read)) => assert_eq!(read.subject, document.hash),
            other => panic!("expected a tag, got {other:?}"),
        }
    }
}
