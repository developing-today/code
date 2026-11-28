//! Document store API for world sessions (split from the parent
//! module, #8594): the public-document name rules, the paging
//! selector, and the store reads behind the session layer's document
//! frames and the SSH shell's listing. The session glue that serves
//! these over frames stays in the parent module.

use futures_lite::StreamExt as _;
use iroh_blobs::api::Store;

use super::{
    JoinCapability, MAX_RECORDS_PAGE_BYTES, ServerFrame, SessionIo, WorldHandle, WorldMetrics,
    send_json,
};

pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_DOCUMENT_LIST_BYTES: usize = 32 * 1024;
pub(crate) const MAX_DOCUMENT_LIST_ITEMS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentError {
    InvalidName,
    Missing,
    TooLarge,
    Binary,
    Unavailable,
}

impl DocumentError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::InvalidName => "document name must be a public .txt, .md, or .markdown tag",
            Self::Missing => "document not found",
            Self::TooLarge => "document exceeds the 64 KiB SSH viewing limit",
            Self::Binary => "document is not valid UTF-8 text",
            Self::Unavailable => "document unavailable",
        }
    }
}

pub(crate) fn is_viewable_document_name(name: &str) -> bool {
    if !is_public_document_name(name) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt")
}

pub(crate) fn is_public_document_name(name: &str) -> bool {
    !name.is_empty()
        && !crate::tags::is_internal_tag(name)
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
}

/// All public document names in the store, sorted, filtered by
/// `prefix` and continued after `after` (exclusive). The session
/// layer owns the serving rules (D1-040 decision 7); renderers only
/// format the names they are given.
pub(crate) async fn list_document_names(
    store: &Store,
    prefix: Option<&str>,
    after: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut tags = store.tags().list().await?;
    let mut names = Vec::new();
    while let Some(item) = tags.next().await {
        let item = item?;
        let Ok(name) = std::str::from_utf8(item.name.as_ref()) else {
            continue;
        };
        if !is_public_document_name(name) || prefix.is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        if after.is_some_and(|a| name <= a) {
            continue;
        }
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}

/// The on-wire size of one name in a `documents` frame: its JSON
/// string encoding (quotes and escapes included) plus the separator.
/// Raw byte length is not enough — a quote-heavy name grows when
/// serialized, and the frame budget is a serialized budget.
pub(crate) fn document_name_wire_size(name: &str) -> usize {
    serde_json::to_vec(name).map_or(name.len() + 2, |bytes| bytes.len()) + 1
}

/// One page of document names under the records byte-page rule: the
/// page's `next_after` is the last name *included*, so the exclusive
/// continuation never skips a name.
pub(crate) fn document_names_page(
    names: &[String],
) -> Result<(Vec<String>, Option<String>), &'static str> {
    let mut page = Vec::new();
    let mut used = 0usize;
    let mut more = false;
    for name in names {
        let size = document_name_wire_size(name);
        if used + size > MAX_RECORDS_PAGE_BYTES {
            if page.is_empty() {
                return Err("a document name is too large to serve in one page");
            }
            more = true;
            break;
        }
        used += size;
        page.push(name.clone());
    }
    let next_after = if more { page.last().cloned() } else { None };
    Ok((page, next_after))
}

/// Bounded selection of one documents page from a name stream
/// (Codex P2 on PR #7678): collecting and sorting the whole store to
/// serve a bounded page does not scale, so candidates are held in a
/// max-heap whose serialized size stays within the page budget;
/// larger names are evicted as smaller ones arrive. Eviction is
/// tracked so the continuation cursor never skips an evicted name:
/// the cursor only ever names a served name that sorts before every
/// evicted name, or the last served name below the smallest evicted
/// name.
#[derive(Default)]
pub(crate) struct DocumentPageSelector {
    heap: std::collections::BinaryHeap<String>,
    heap_bytes: usize,
    min_evicted: Option<String>,
}

impl DocumentPageSelector {
    pub(crate) fn push(&mut self, name: String) {
        self.heap_bytes += document_name_wire_size(&name);
        self.heap.push(name);
        while self.heap_bytes > MAX_RECORDS_PAGE_BYTES && self.heap.len() > 1 {
            let evicted = self.heap.pop().unwrap_or_default();
            self.heap_bytes -= document_name_wire_size(&evicted);
            if self
                .min_evicted
                .as_deref()
                .is_none_or(|min| evicted.as_str() < min)
            {
                self.min_evicted = Some(evicted);
            }
        }
    }

    pub(crate) fn finish(self) -> Result<(Vec<String>, Option<String>), &'static str> {
        let mut candidates: Vec<String> = self.heap.into_vec();
        candidates.sort();
        let (mut page, mut next_after) = document_names_page(&candidates)?;
        if let Some(min_evicted) = &self.min_evicted {
            if page.iter().any(|name| name.as_str() > min_evicted.as_str()) {
                // A served name sorts after an evicted one: resuming after
                // it would skip the evicted name. Serve only the names
                // below the smallest evicted name; the rest follow on the
                // continuation.
                let cut = page.partition_point(|name| name.as_str() < min_evicted.as_str());
                page.truncate(cut);
                next_after = page.last().cloned();
            } else if next_after.is_none() {
                // The whole candidate set was served, but evicted names
                // follow it.
                next_after = page.last().cloned();
            }
        }
        Ok((page, next_after))
    }
}

/// Read one public document as (byte size, UTF-8 text), enforcing the
/// viewable-name rules and the per-document byte cap server-side.
pub(crate) async fn read_document(
    store: &Store,
    name: &str,
) -> Result<(u64, String), DocumentError> {
    if !is_viewable_document_name(name) {
        return Err(DocumentError::InvalidName);
    }
    let tag = store
        .tags()
        .get(name)
        .await
        .map_err(|_| DocumentError::Unavailable)?
        .ok_or(DocumentError::Missing)?;
    let size = match store.blobs().status(tag.hash).await {
        Ok(iroh_blobs::api::blobs::BlobStatus::Complete { size }) => size,
        _ => return Err(DocumentError::Unavailable),
    };
    if size > MAX_DOCUMENT_BYTES as u64 {
        return Err(DocumentError::TooLarge);
    }
    let bytes = store
        .blobs()
        .get_bytes(tag.hash)
        .await
        .map_err(|_| DocumentError::Unavailable)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| DocumentError::Binary)?;
    Ok((size, text.to_owned()))
}

/// The rendered listing the SSH shell has always produced, kept as a
/// parity reference for the frame-served listing (D1-040 test plan).
pub(crate) async fn list_public_documents(
    store: &Store,
    prefix: Option<&str>,
) -> anyhow::Result<String> {
    let mut names = Vec::new();
    let mut total_bytes = 0usize;
    let mut truncated = false;
    for name in list_document_names(store, prefix, None).await? {
        let line = format!("{name}\n");
        if names.len() >= MAX_DOCUMENT_LIST_ITEMS
            || total_bytes + line.len() > MAX_DOCUMENT_LIST_BYTES
        {
            truncated = true;
            break;
        }
        total_bytes += line.len();
        names.push(name);
    }
    let mut output = String::from("public named files:\n");
    if names.is_empty() {
        output.push_str("(none)\n");
    } else {
        for name in names {
            output.push_str(&name);
            output.push('\n');
        }
    }
    if truncated {
        output.push_str("[listing truncated]\n");
    }
    output.push_str("View UTF-8 .txt/.md/.markdown files with /document <name>.\n");
    Ok(output)
}

/// The rendered read the SSH shell has always produced: the header is
/// a rendering each client formats from the `document` frame's fields.
pub(crate) async fn read_public_document(
    store: &Store,
    name: &str,
) -> Result<String, DocumentError> {
    let (size, text) = read_document(store, name).await?;
    Ok(format!("--- {name} ({size} bytes; read-only) ---\n{text}"))
}

pub(crate) async fn documents_session<I: SessionIo>(
    meter: &WorldMetrics,
    world: &WorldHandle,
    io: &mut I,
    token: JoinCapability,
    store: Option<&Store>,
    prefix: Option<&str>,
    after: Option<&str>,
) -> Result<(), &'static str> {
    if world.participant_id(token).await.is_err() {
        return Err("join denied");
    }
    let Some(store) = store else {
        return Err("documents unavailable");
    };
    let mut tags = store
        .tags()
        .list()
        .await
        .map_err(|_| "documents unavailable")?;
    let mut selector = DocumentPageSelector::default();
    while let Some(item) = tags.next().await {
        let item = item.map_err(|_| "documents unavailable")?;
        let Ok(name) = std::str::from_utf8(item.name.as_ref()) else {
            continue;
        };
        if !is_public_document_name(name) || prefix.is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        if after.is_some_and(|a| name <= a) {
            continue;
        }
        selector.push(name.to_owned());
    }
    let (names, next_after) = selector.finish()?;
    if !send_json(meter, io, &ServerFrame::Documents { names, next_after }).await {
        return Err("connection closed");
    }
    Ok(())
}

pub(crate) async fn document_session<I: SessionIo>(
    meter: &WorldMetrics,
    world: &WorldHandle,
    io: &mut I,
    token: JoinCapability,
    store: Option<&Store>,
    name: &str,
) -> Result<(), &'static str> {
    if world.participant_id(token).await.is_err() {
        return Err("join denied");
    }
    let Some(store) = store else {
        return Err("documents unavailable");
    };
    match read_document(store, name).await {
        Ok((size, text)) => {
            if !send_json(
                meter,
                io,
                &ServerFrame::Document {
                    name: name.to_owned(),
                    size,
                    text,
                },
            )
            .await
            {
                return Err("connection closed");
            }
            Ok(())
        }
        Err(error) => Err(error.message()),
    }
}
