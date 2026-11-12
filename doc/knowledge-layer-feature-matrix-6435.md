# Knowledge-layer feature matrix — issue #6435 (program #5678)

Status: investigation deliverable for issue #6435, program tracker issue #5678.
Observation date: 2026-11-08. Product facts were observed on 2026-11-08
from the sources in §7; products change, so re-observe before relying on
any cell for a build decision.

## 1. Method and provenance

- Enumeration: for each in-scope product, walk the vendor's own
  documentation and product pages against the normalized feature
  vocabulary in §2. Record presence, depth, and meaningful
  implementation differences — not checkbox presence alone.
- Evidence types (every inventory entry and matrix cell carries one):
  - primary — vendor documentation / official product or help pages.
  - hands-on — direct use of the product by the investigator.
  - secondary — third-party reporting; used only where flagged.
- This pass is documentation-only: evidence is primary throughout,
  except where a cell is explicitly marked secondary or unknown.
  Whether hands-on evaluation is funded per product is one of Drew's
  open program questions (§6) — recorded as open, not assumed.
- Editions: facts describe the generally available product as
  documented on the observation date. Where free vs. paid editions
  differ in a way that affects a cell, the difference is noted; the
  program-level edition question (§6) remains open, so no cell
  silently assumes a paid tier.
- Unknowns are recorded as unknown with the gap named. Nothing is
  inferred from a competitor's marketing category or from another
  product's behaviour.
- Open Knowledge: §3.9 — identity unconfirmed; recorded as an explicit
  identity blocker per the acceptance criteria of issue #6435, with
  the most plausible candidate noted as unverified.

## 2. Normalized feature vocabulary

Vendor feature names differ; the matrix uses these normalized terms:

- Data model — the primary unit (file, page, block, object,
  notebook/section) and how units nest or link.
- Storage locus — local files, vendor cloud, or user-chosen backend.
- Sync — how changes move between devices, and conflict behaviour
  where documented.
- Sharing / permissions — who can be granted read or write, and at
  what granularity (object, container, workspace).
- Export / import — formats out (and fidelity limits) and formats in.
- Lock-in — what is lost on exit: format fidelity, metadata,
  attachments, history; and whether exit is paywalled or manual.
- Graph — backlinks, link graph or visualisation, embeds/transclusion.
- AI over notes — vendor AI features that read note content, and what
  content they can reach.
- Extensibility — plugins, API, automation.
- Attachments and media — file handling, limits, OCR/indexing.
- Capture — web clipper, mobile share, email-in, quick capture.
- Offline — what works with no network.

Read-authority note (applies to every row): the knowledge layer's rule
is already decided and is not re-opened here — resolution order is
object -> user default -> server config -> public (issue #6435).
Findings in §5 describe each product's sharing model against that
order and never recommend a read path that exposes content the
resolved policy denies.

## 3. Product inventories

All entries: evidence primary, observed 2026-11-08, except where
marked unknown or secondary.

### 3.1 Obsidian

- Data model / storage: a vault is a folder on the local file system;
  notes are plain-text Markdown files ("durable and not locked into a
  proprietary format"). Canvas and Bases (database-like views over
  files/properties) are core plugins.
- Sync: optional paid Obsidian Sync — end-to-end encrypted (AES-256),
  version history, selective sync, merges on reconnect. Because the
  vault is plain files, third-party file sync also works.
- Sharing / permissions: via Sync shared vaults only — max 20
  collaborators, every collaborator needs an active Sync
  subscription, no fine-grained permissions (collaborators get the
  owner's permissions except inviting), no live co-editing of the
  same file (concurrent edits merge during sync). Publish produces
  read-only web sites.
- Export / import: no export step — notes already are Markdown files
  in a folder the user owns. The official Importer plugin imports
  from Notion, Evernote, OneNote, Apple Notes/Journal, Google Keep,
  Airtable, HTML, CSV, Markdown, Textbundle and others.
- Pricing / lock-in: app free without limits, including commercial
  use. Sync Standard USD 4/user/mo billed annually (1 vault, 1 GB,
  5 MB/file, 1-month history); Sync Plus USD 8 (10 vaults, 10 GB
  upgradable to 100 GB, 200 MB/file, 12-month history). Publish price:
  unknown in this pass. Content lock-in is minimal; paid lock-in is
  only the Sync/Publish convenience.
- Graph: core Graph view (notes as nodes, internal links as edges),
  Backlinks (linked and unlinked mentions), Outgoing links.
- AI over notes: no native AI question-answering or semantic search
  over the vault is documented on the primary pages checked
  (unknown / not documented). The only documented first-party AI is
  the Web Clipper Interpreter — bring-your-own LLM provider
  (including local models) to extract or summarise a web page being
  clipped, not to query the vault. Vault AI is via community plugins.
- Extensibility: core plugins plus thousands of community plugins and
  themes via an open plugin API. No general hosted REST API for vault
  content documented (unknown) — extensibility is in-app, because the
  vault is local.
- Attachments: files live in the vault folder and embed inline.
  Sync file-size caps are plan-dependent (above).
- Capture: official Web Clipper extension (template-based article
  extraction, highlights); images are linked by URL, not
  auto-downloaded. Mobile apps exist; mobile share-sheet specifics
  unknown from primary pages opened.
- Offline: fully offline-first; Sync merges when back online.

### 3.2 Evernote

- Data model / storage: a note belongs to one notebook or one space;
  notebooks group into stacks (not shareable as a unit); spaces are
  project workspaces containing notes and notebooks, shareable in
  whole or part; plus tags. Cloud-first: local notebooks are no
  longer supported in apps v10+.
- Sync: cloud sync; Starter allows up to 3 synced devices, Advanced
  unlimited.
- Sharing / permissions: share a space, notebook, or notes by invite
  with view vs. edit set at invite; a single note can be shared via
  public URL (anyone with the URL views the current note).
  Finer per-note tiers: partially verified, unknown in full.
- Export / import: export is desktop-app only (not web): ENEX
  (Evernote XML) or single-/multi-page HTML, plus PDF (single note,
  up to 100 notes, or a whole notebook as one PDF; PDF excludes the
  actual attachment files — use ENEX/HTML for those). At most 100
  selected notes at a time, or entire notebooks. Free users can
  export; no paid subscription is needed. Import is ENEX only per
  the help pages, and notebook structure is not preserved (notes
  land in an Import notebook).
- Pricing / lock-in: Starter USD 14.99/mo or 99/yr (1,000 notes,
  20 notebooks, 1,000 attachments, 10 spaces, 100 tags, 5 GB,
  3 devices); Advanced USD 24.99/mo or 249.99/yr (unlimited notes,
  notebooks, attachments, storage, devices, subject to safeguard
  limits). Export is not paywalled, but the format is proprietary
  ENEX-first and bulk export is manual, per-notebook, desktop-only —
  that is the practical lock-in.
- Graph: unknown — no graph view or backlinks feature found in the
  primary documentation checked.
- AI over notes (v11): AI Assistant (built with OpenAI; chat over the
  open note or the whole account, semantic note search, web search);
  Semantic Search (find by meaning; indexing on Evernote servers,
  including attachment text via OCR); AI Meeting Notes; AI
  Edit/Transcribe. Evernote states content is not used to train AI
  models; AI features are on by default and individually
  disable-able.
- Extensibility: a legacy developer platform exists (Evernote Cloud
  API). Current non-legacy API availability and terms: unknown.
- Attachments: count against plan caps (above); included as note
  resources in ENEX/HTML exports. Per-file size limits unknown in
  this pass.
- Capture: Web Clipper extension (article / full page / bookmark /
  custom clips, notebook and tags chosen at clip time); both paid
  tiers list web, mobile, and email capture.
- Offline: Offline Notebooks on mobile (selected notebooks download
  for offline use). Whether offline download is plan-gated in the
  current plans: unknown.

### 3.3 Notion

- Data model / storage: a cloud-hosted workspace of pages made of
  blocks (a block is any single piece of content); databases whose
  rows are pages, with properties, relations, and views. Everything
  is stored in the vendor cloud; no local-file or self-host model is
  documented.
- Sync: inherent to the hosted workspace; offline edits sync on
  reconnect.
- Sharing / permissions: page-level sharing with members, groups,
  teamspaces, and guests (invited by email to specific pages; guests
  see only pages they are invited to). Permission levels: Full
  access, Can edit, Can edit content (databases only — rows and
  properties, not database structure or views), Can comment, Can
  view. General access can be invite-only, workspace-wide, or anyone
  on the web with the link; publish-to-web / Notion Sites also
  exists. Business and above add private teamspaces, permission
  groups, and granular database permissions.
- Export / import: a page or database exports as PDF, HTML, or
  Markdown and CSV (non-database pages to Markdown; full-page
  databases to CSV plus Markdown subpages; callouts export as HTML).
  Whole-workspace export (HTML/Markdown/CSV plus uploaded files) is
  desktop/web only, delivered by an email or in-app link expiring
  after 7 days and can take up to 30 hours; private pages the
  exporter cannot access are excluded; PDF with subpages requires
  Business/Enterprise; Enterprise can disable export. Notion states
  an exported workspace cannot be re-uploaded to instantly recreate
  it. Import (desktop/web): txt, md, docx, csv, html, pdf, zip, plus
  direct importers including Evernote and Confluence.
- Pricing / lock-in: plans are per member (guests free). Free: pages
  and blocks limited when a workspace has 2+ members, file uploads
  up to 5 MB, 7-day page history, 10 guests. Paid: uploads up to
  5 GB/file, longer history. Per-member USD list prices for
  Plus/Business: unknown in this pass. Notion AI is a trial on
  Free/Plus and included at Business; Custom Agents are free to try,
  then USD 10 per 1,000 monthly Notion credits. Lock-in is fidelity,
  not access: export exists, re-import does not recreate the
  workspace.
- Graph: backlinks yes — created automatically when a page is
  mentioned, listed under the page title/properties, and visible
  only for pages the viewer can access. A graph visualisation:
  unknown — none documented in the primary help checked.
- AI over notes: Notion AI — chatbot, writing help, and smart search
  across the workspace and connected apps (e.g. Slack, Google
  Drive); question-answering from workspace content, summarise,
  draft, database autofill, AI Meeting Notes, Enterprise Search,
  Research mode, Notion Agent / Custom Agents (Business/Enterprise).
  Notion states AI does not train on customer data unless opted in.
- Extensibility: a public API available to all users (databases,
  pages, content; internal and public connections, tokens/OAuth), a
  connections gallery, database automations, webhooks, embeds, and
  Workers (beta, Business/Enterprise) to run custom code. In-app
  third-party UI plugins in the Obsidian sense: not documented —
  extensibility is API/connections-first.
- Attachments: images, video, audio, files in pages; Free up to
  5 MB/file, paid up to 5 GB/file.
- Capture: Notion Web Clipper (Chrome and Safari extensions; save a
  page to a chosen workspace/page/database); mobile capture uses the
  native iOS/Android share sheet.
- Offline: desktop/mobile apps only, not web browsers. Individual
  pages can be marked Available offline on all plans; paid plans
  auto-download recently visited and favourited pages. Databases
  download only the first 50 rows of the first view; embeds, AI
  blocks, forms, and buttons need a connection; sharing and
  permission edits cannot be done offline; downloads are per-device.

### 3.4 Microsoft OneNote

- Data model / storage: Notebook -> Sections / Section Groups ->
  Pages; on disk, sections are .one / .onetoc2 files. The Microsoft
  Graph model exposes exactly notebooks, sections, sectionGroups,
  pages.
- Local vs cloud / sync / offline: cloud-first for sharing and sync —
  notebooks live on OneDrive or SharePoint and auto-sync (change
  level; notebook folders must not be synced by file-sync tools). A
  local copy is kept for offline editing and merges on reconnect. A
  local notebook can be moved to OneDrive, after which the local
  copy no longer syncs.
- Sharing / permissions: the notebook must be on OneDrive or
  SharePoint to share. Share Entire Notebook by invite or link, per
  recipient Can edit or Can view, revocable. A section link exposes
  the entire notebook. Single-page sharing was removed — previously
  shared page links have expired; the workaround is a static
  snapshot/email or a separate shared notebook.
- Export / import: web export downloads a notebook as a ZIP, only
  for notebooks on personal OneDrive — not for work/school OneDrive
  or SharePoint notebooks. Notebook import is deprecated. Desktop
  File > Export exports the current notebook/section/page as a
  OneNote Package (.onepkg) or PDF only. Export to Word or Markdown
  from a primary page opened in this pass: unknown.
- Pricing / lock-in: dollar pricing from a primary pricing page:
  unknown in this pass. Verified gating: Copilot requires a licence
  (below). The export asymmetry above (personal-OneDrive-only web
  export) is the main lock-in signal.
- Graph: unknown / not documented — no graph view or backlink model
  on any primary page checked.
- AI over notes (Copilot): summarise (section, selection, topic),
  rewrite (tone, style, formality, length), create task lists,
  analyse ideas, draft plans, chat pane. Licence: Microsoft Copilot
  (Work) licence for OneNote for Microsoft 365 on Windows, Mac,
  iPad, and web; or an eligible Microsoft 365 subscription in
  OneNote for Windows only.
- Extensibility: Microsoft Graph OneNote REST API (delegated auth
  only; app-only authentication is not supported). In-app add-ins
  from a primary page: unknown.
- Attachments: Insert > File Attachment places a copy of the file on
  the page as an icon; File Printout places an annotatable printout.
  Notebooks grow large from embedded documents, scans, PDFs, and
  media clips.
- Capture: OneNote Web Clipper (free): full page, region screenshot,
  article as editable text and images, PDF page ranges, YouTube,
  targeted to a notebook/section on OneDrive. Native apps for
  Windows/Mac/iOS/Android/Web.
- Offline: yes, via the local copy (above).

### 3.5 NotebookLM — now Gemini Notebook

Naming (primary, observed 2026-11-08): on 2026-07-16 Google renamed
NotebookLM to Gemini Notebook, "the same standalone product".
Enterprise (Google Cloud) facts below are labelled enterprise; Google
states the enterprise product differs from the consumer one, and the
two are not interchangeable.

- Data model: Notebook -> Sources + Notes / Studio outputs + Chat.
  A notebook is a collection of imported sources; chat and Studio
  outputs are generated from them. There is no linked page/block
  model in the primary docs checked.
- Local vs cloud / sync: cloud, Google-account product, with full
  cross-app sync between the Gemini app and the standalone Gemini
  Notebook experience. A local-file storage model: unknown — none is
  described in primary docs.
- Sharing / permissions: consumer sharing is documented at the
  whole-notebook unit (e.g. a Mind Map is shared by sharing the
  notebook). Consumer Viewer/Editor role definitions: unknown in
  this pass. Enterprise: share by email/group within the same
  project/workforce pool with the required licence; Viewer
  (interact only; cannot upload sources or add notes) and Editor
  (owner-like except cannot delete notebooks, share notebooks, or
  revoke access); sharing is revocable.
- Export / import: import is adding sources (below). Export verified
  from primary pages opened: Mind Map download as a file, and
  share-by-notebook. A general notebook or source export format is
  not documented on the primary pages opened (unknown) — the absence
  of a documented whole-notebook export is itself the lock-in
  signal.
- Pricing / lock-in: a free tier exists (free of charge in Workspace
  for Education Fundamentals; upgrade for higher limits and premium
  features). Tier names Standard / Plus / Pro / Ultra are verified
  (e.g. Mind Map limits 10 / 20 / 100 / 500 / 1,000 per day
  respectively). Consumer dollar prices and notebook/source caps
  from a primary page: unknown in this pass.
- Graph: no backlinks or page graph. Closest feature: Mind Maps —
  a branching diagram of main topics and related ideas from the
  uploaded sources; nodes are selectable to ask chat questions.
- AI over notes: the product's core — source-grounded chat,
  summaries, and Studio generation (Mind Maps verified; code
  execution grounded in sources).
- Extensibility: enterprise only, verified — a REST API for
  notebooks/sources on Discovery Engine, requiring a Google Cloud
  project. A consumer API or plugin system: unknown / none
  documented.
- Attachments (as sources), enterprise list: documents (.pdf, .txt,
  .md, .docx, .pptx, .xlsx), audio (.mp3, .m4a, .wav, .aac, .ogg,
  .opus, .mp4, .mpeg, .3gp and more), images (.png, .jpg, .jpeg),
  plus Google Docs/Slides, raw text, web content, and YouTube videos
  as source types. The consumer list may differ.
- Capture: web and mobile apps; create notebooks and upload sources.
  A web clipper extension: unknown — none in primary docs.
- Offline: unknown from primary sources in this pass.

### 3.6 Logseq

Two coexisting models, both verified in primary docs dated 2026:
file graphs (original Logseq) and DB graphs (current Logseq, beta).
The project README warns for the DB version that the mobile app and
RTC are alpha and data loss is possible; automated/SQLite backups are
recommended.

- Data model: Graph -> Pages / Blocks as a block outliner (Markdown
    or Org-mode support). The DB version unifies pages and blocks as
    Nodes (a node is a page or a block; the two now behave similarly),
    referenced with [[]], with properties, tags that can carry
    inherited tag properties, journals, tasks, queries,
    cards/flashcards, templates, and a Library for namespaced pages.
- Local vs cloud: local-first / privacy-first. File graphs are
  Markdown/Org files; DB graphs are backed by SQLite (graph export
  is a .db file; automated backups keep the last 12 in a backups
  folder inside the graph directory on desktop). Logseq Sync is
  optional, not required for core use.
- Sync: Logseq Sync is RTC (real-time collaboration) for DB graphs;
  setup requires an encryption password; a self-hosted sync URL can
  be configured. File-graph sync via third-party folder sync from a
  primary page in this pass: unknown.
- Sharing / permissions: no granular per-page role model in the
  primary docs checked (unknown). Verified routes: RTC collaboration
  via Sync, and Logseq Publish — publish pages to logseq.io,
  read-only, optionally password-protected (password-protected pages
  are excluded from search), with Unpublish; Publish is a paid
  feature requiring a Sync account and can be self-hosted.
- Export / import (DB graphs, verified in detail): export graph to
  SQLite .db; SQLite + assets (.zip); EDN (the only export that
  fully captures a graph's data and is editable — but not yet
  recommended as the only backup); standard Markdown without block
  properties (explicitly lossy: unlikely to ever export timestamps
  or all properties); public pages; debug transit file. Import:
  SQLite, SQLite+assets, file-to-DB graph from Markdown (remapping
  task statuses and tags, copying assets), EDN, debug transit.
  File-graph (OG) export list from a primary page in this pass:
  unknown.
- Pricing / lock-in: the team states all local features will be free;
  only server-reliant features (real-time collaboration) are
  charged. Sync pricing in the DB doc: paid, for Open Collective
  Sponsors USD 15/month and Backers USD 5/month; self-hosting is an
  alternative. The data stays in local Markdown/SQLite/EDN, so paid
  sync does not lock in the data. The app is open source, supported
  via Open Collective.
- Graph: nodes have Linked References and Unlinked References
    sections; [[]] page/node references and block embeds. A visual
    graph-view claim from a primary page in this pass: unknown (the
    references model is verified; the visualisation is not stated on
    the pages opened).
- AI over notes: no built-in AI chat or summarisation is documented
  in the primary README/DB doc. AI access is via an optional MCP
  Server (enable in Settings > AI, HTTP-server auth token): AI apps
  can connect to the current/local graph to search, create and list
  pages, tags and properties, and create/edit blocks, with a
  dry-run mode and undo/redo. Native AI beyond MCP/plugins: unknown.
- Extensibility: a Plugin API plus themes; for DB graphs, 65+
  plugins that support DB graphs via the marketplace, a JS Plugin
  SDK and a CLJS SDK; also a Logseq CLI and the MCP/HTTP server.
- Attachments: assets — drag a file onto a block, upload via the
  Asset page, or create from an external image URL or local path;
  asset files are stored under the graph's assets/ directory.
  Built-in PDF annotation.
- Capture: the DB iOS app (invite-only/alpha per the DB doc) has a
  Capture tab (create blocks, voice recording, send captures to
  today's journal) and external apps can share text into today's
  journal. A browser web clipper from a primary page: unknown.
  Android DB app: coming soon / alpha per the README.
- Offline: core use is on-device (local file/SQLite graphs, sync
  optional). An explicit vendor offline-mode statement: unknown.

### 3.7 Joplin

- Data model / storage: Markdown notes and to-dos in nested
  notebooks/sub-notebooks plus flat tags; attachments are resources.
  Notes and resources are stored locally in a SQLite database (not
  plain files — export produces files).
- Local vs cloud: local-first / offline-first; cloud is optional,
  only for sync. Desktop (Windows/macOS/Linux), mobile (iOS/Android),
  terminal, and a web app (Joplin Cloud only).
- Sync: targets include Joplin Cloud, Nextcloud, S3, WebDAV,
  Dropbox, OneDrive, and the local filesystem — designed without a
  hard dependency on any particular service, and easy to switch.
  End-to-end encryption is available in all apps for all targets but
  is opt-in (enable on one device first with a master password that
  cannot be recovered); it encrypts notes and resources before they
  leave the device; the local database is not encrypted by it. No
  peer-to-peer sync.
- Sharing / permissions: only via Joplin Cloud / Joplin Server —
  publish a note to a public link; share or collaborate on a
  notebook (sharing to others requires Pro or above; Basic can only
  join). Read-only vs. edit share permissions exist only on Teams /
  Server Business. No per-note permissions with self-hosted or
  third-party sync.
- Export / import: export JEX (full-fidelity, re-importable,
  preserves tags and metadata), Markdown (plus a frontmatter
  variant), HTML, PDF, JSON/raw. Import ENEX (as Markdown or HTML),
  Markdown files/directories, JEX, HTML, OneNote ZIP/.one.
- Pricing / lock-in: the app is free and open source (AGPL-3.0).
  Joplin Cloud is optional — Basic EUR 2.99/mo (2 GB, 10 MB/note),
  Pro EUR 5.99/mo (30 GB, 200 MB/note), Teams EUR 7.99/user/mo
  (50 GB); yearly billing is cheaper. Lock-in is low: free sync via
  the user's own storage plus Markdown/JEX export. (Joplin Server's
  own licence terms for self-hosting differ from the app's; check
  before commercial self-hosting.)
- Graph: unknown (native) — no native graph view or backlink feature
  found in the primary help/plans pages checked; community plugins
  provide graph visualisation via the plugin system.
- AI over notes: Joplin Cloud AI (beta, on Cloud plans) — a hosted
  chat model in the desktop app that can summarise, rewrite, or
  answer questions about the user's notes. No built-in local/offline
  AI documented. Other AI is via plugins (unknown as a core feature).
- Extensibility: a JavaScript/TypeScript plugin system and Plugin
  API, custom themes, and a Data/REST API (the clipper server,
  token-authenticated) exposing notes, notebooks, tags, and
  resources; plugins can register import/export modules.
- Attachments: images, photos, videos, audio, PDFs and documents as
  resources; OCR search inside PDFs and images. Size caps are a
  Cloud-plan limit, not an app limit on the user's own sync.
- Capture: Web Clipper for Chrome/Firefox (full page, simplified
  page, selection, screenshot, URL); email-to-note (Pro and above);
  mobile handwriting/drawing, speech-to-text, camera/photos; to-dos
  with reminders.
- Offline: yes — fully works offline; sync runs in the background
  when the app is running and connectivity returns.

### 3.8 Anytype

- Data model / storage: an object graph, not folders/files —
  everything is an Object with a Type (schema), Relations/Properties
  (attributes and links), and content Blocks; organised in Spaces;
  Sets are saved queries over all objects of a type, Collections are
  manually curated objects of any type. Local storage is an
  app-managed object database — not plain Markdown files on disk.
- Local vs cloud: local-first — stored on-device first, works
  offline, end-to-end encrypted. No web version of the classic app.
  Desktop and iOS/Android. (A successor, Anytwo, was announced in
  September 2026 with a different, undecided membership model; the
  pricing below is classic Anytype.)
- Sync: the Any-Sync protocol — end-to-end / zero-knowledge
  encryption by default, keys from a 12-word recovery phrase, the
  vendor cannot decrypt. Modes: the Anytype Network (encrypted
  backup nodes plus direct device sync), peer-to-peer over LAN
  (mDNS discovery), a self-hosted Any-Sync network, and local-only.
  Membership changes need the network; other writes work offline.
- Sharing / permissions: Shared Spaces via invite link; roles at
  space level only — Owner / Editor (edit all objects) / Viewer
  (read-only). No per-object permissions: members of a shared space
  see everything in it; sharing is all-or-nothing per space.
  Publish-to-web for a page also exists.
- Export / import (desktop only): export Markdown, Any-Block (JSON
  or Protobuf — full fidelity, re-importable), PDF, HTML; chats and
  space members are not exported. Import from Notion (API importer),
  Obsidian, Evernote, Markdown, HTML, TXT, CSV, Any-Block/Protobuf.
- Pricing / lock-in: free to use locally — free tier has 100 MB of
  remote (backup-node) storage (only files count), 10 shared spaces,
  unlimited private spaces. Plus USD 5/mo (1 GB), Pro USD 10/mo
  (10 GB), Ultra USD 20/mo (100 GB); yearly billing is 20% less.
  Self-hosting and local-only need no membership. Lock-in is medium:
  Markdown export exists but loses relations and structure relative
  to Any-Block; the data is not a folder the user can simply open in
  another editor. If a membership lapses, backup-node backups are
  deleted after 30 days; local data remains. (Pricing figures:
  secondary — third-party pages reporting the vendor pricing page,
  checked 2026-10-19 and 2026-11-04; the rest of this section is
  primary.)
- Graph: yes, native — Graph Mode visualises links and relations
  between objects; relations are bidirectional in effect; Sets,
  Collections, inline mentions, and link blocks create links.
- AI over notes: unknown (built-in) — no built-in AI assistant or
  chat found in the primary docs/pages checked for classic Anytype.
  External AI access exists via the official local API and MCP
  server, letting MCP clients search, create, and export objects as
  Markdown. Anytype should not be listed as AI-native.
- Extensibility: a local HTTP API (spaces, objects, types,
  templates, properties), a CLI, and an MCP server; gallery/templates
  and custom Types/Relations. A Joplin-style third-party plugin
  ecosystem: unknown — no primary evidence of a comparable plugin
  marketplace.
- Attachments: file/image/video objects attached to objects; files
  are what count toward the remote-storage quota; file blocks are
  encrypted on file/backup nodes or self-hosted storage.
- Capture: the official Anytype Web Clipper (Chrome): save a whole
  page or highlighted content, assign a Type, tags/status/relations,
  and Collections; mobile share-sheet for URLs/objects; Bookmark
  objects.
- Offline: yes — fully offline-capable; changes sync on reconnect or
  over LAN.

### 3.9 Open Knowledge — identity blocker (unconfirmed)

No single product is confirmable from the name alone in a
note-taking context. This row is therefore blocked, per the
acceptance criteria of issue #6435, until Drew identifies the
product (§6, question 3). Candidates, in plausibility order
(evidence secondary except where a candidate's own docs are cited):

1. OpenKnowledge by Inkeep (one word) — an open-source, local-first,
   AI-first Markdown knowledge IDE launched on GitHub in June 2026.
   It fits a 2026 competitor list alongside Joplin and Anytype. Its
   own documentation (primary for the candidate) states: plain
   .md/.mdx files, no database, filesystem plus Git as persistence,
   no lock-in; computed backlinks and a link graph; an MCP engine so
   agent tools co-edit the same files; real-time CRDT collaboration
   in the editor; sharing and sync via Git/GitHub rather than a
   proprietary cloud. Its pricing/licence, attachments, and capture
   rows are unknown from primary sources in this pass.
2. Google Cloud Open Knowledge Format (OKF) — a format/spec, not an
   app: vendor-neutral Markdown plus YAML frontmatter bundles,
   published in 2026. It has no sync, sharing, pricing, or UI to put
   in a product matrix, though it could be the intended knowledge
   layer reference.
3. Open Knowledge Foundation (OKFN) — a nonprofit promoting open
   data and content; "open knowledge" is its generic concept, not a
   note-taking product.
4. Open Knowledge Maps — a nonprofit research-discovery tool; not
   personal knowledge management.

Note the spelling: the issue says "Open Knowledge" (spaced), which
does not exactly match candidate 1's "OpenKnowledge", and no link
was supplied to disambiguate. No verified matrix row is published
for this product.

## 4. Product x feature matrix

Cells summarise §3; read the inventory for depth and caveats. Every
cell: evidence primary, observed 2026-11-08, except cells marked
(U) = unknown from primary sources in this pass, and (S) =
secondary. Open Knowledge is omitted — identity blocker, §3.9.

| Feature             | Obsidian                                                    | Evernote                                                 | Notion                                                        | OneNote                                           | NotebookLM / Gemini Notebook                         | Logseq                                                         | Joplin                                                                     | Anytype                                               |
| ------------------- | ----------------------------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------------- | ------------------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------- | -------------------------------------------------------------------------- | ----------------------------------------------------- |
| Data model          | Markdown files in a vault folder; Canvas/Bases views        | Note in one notebook or space; stacks; tags              | Pages of blocks; databases (rows = pages)                     | Notebook > sections > pages (.one files)          | Notebook of sources; generated notes/chat            | Pages/blocks outliner; DB version unifies as Nodes             | Markdown notes/to-dos in nested notebooks; tags                            | Objects with Types, Relations, Blocks; Spaces         |
| Storage locus       | Local files                                                 | Vendor cloud (no local notebooks in v10+)                | Vendor cloud only                                             | OneDrive/SharePoint (local copy for offline)      | Vendor cloud (Google account)                        | Local files (Markdown/Org) or local SQLite                     | Local SQLite DB                                                            | Local object DB                                       |
| Sync                | Optional paid Sync (E2E); or any file sync                  | Cloud; 3 devices (Starter) to unlimited                  | Inherent (hosted)                                             | Auto, change-level, via OneDrive/SharePoint       | Cross-app cloud sync                                 | Optional Sync = RTC; self-hostable                             | Many targets (Cloud, Nextcloud, S3, WebDAV, Dropbox, OneDrive, filesystem) | Any-Sync: network, LAN P2P, self-hosted, local-only   |
| Sharing granularity | Shared vault, all-or-nothing per vault                      | Space/notebook/note invite; single-note public URL       | Page-level, 5 permission levels, guests                       | Whole notebook only (page sharing removed)        | Whole notebook; enterprise Viewer/Editor             | Publish pages read-only; RTC collaboration; per-page roles (U) | Notebook share / note public link, Cloud/Server only                       | Space-level only; no per-object permissions           |
| Export              | None needed (files); importers for rivals                   | ENEX/HTML/PDF, desktop only, <=100 notes or per notebook | PDF/HTML/Markdown+CSV; workspace export by expiring link      | ZIP (personal OneDrive only); .onepkg/PDF desktop | Mind Map download; general export (U)                | SQLite, EDN (full), Markdown (lossy), assets zip               | JEX (full), Markdown, HTML, PDF, JSON                                      | Markdown, Any-Block (full), PDF, HTML                 |
| Lock-in             | Minimal (plain files)                                       | Medium-high: ENEX-first, manual bulk export              | Medium: export exists, re-import does not recreate            | Medium-high: export asymmetry by account type     | High: no documented whole-notebook export            | Low: local data, EDN/SQLite/Markdown out                       | Low: own-storage sync + Markdown/JEX                                       | Medium: Markdown loses relations; Any-Block is native |
| Graph / backlinks   | Graph view + backlinks (core)                               | (U)                                                      | Backlinks (access-scoped); graph view (U)                     | (U) / not documented                              | Mind Maps over sources; no backlinks                 | Linked/unlinked references; embeds; visual graph (U)           | (U) native; plugins provide                                                | Native graph over objects/relations                   |
| AI over notes       | None native over vault (U); clipper Interpreter (BYO model) | AI Assistant + semantic search over account (hosted)     | Notion AI search/Q&A over workspace + connected apps (hosted) | Copilot, licence-gated (hosted)                   | Core product: source-grounded chat + Studio (hosted) | None built in; MCP server exposes graph to external AI         | Cloud AI beta (hosted, Cloud plans)                                        | None built in (U); local API + MCP for external AI    |
| Extensibility       | Core + community plugins, open API                          | Legacy API; current API (U)                              | Public API, connections, automations, Workers (beta)          | Graph REST API (delegated only)                   | Enterprise REST API only                             | Plugin API + marketplace, CLI, MCP                             | Plugins, REST Data API                                                     | Local HTTP API, CLI, MCP; plugin marketplace (U)      |
| Capture             | Web Clipper; mobile apps                                    | Web Clipper; web/mobile/email capture                    | Web Clipper (Chrome/Safari); mobile share sheet               | Web Clipper (free)                                | Upload sources in web/mobile apps; clipper (U)       | iOS Capture tab (alpha); share into journal; clipper (U)       | Web Clipper; email-to-note (Pro+); mobile capture                          | Web Clipper; mobile share sheet                       |
| Offline             | Full                                                        | Selected notebooks on mobile; plan gating (U)            | Downloaded pages only; DBs truncated to 50 rows               | Full via local copy                               | (U)                                                  | Core use on-device; explicit statement (U)                     | Full                                                                       | Full                                                  |

## 5. Findings — traced to the cluster designs #5837-#5844

For each cluster: the matrix evidence, then adopt / adapt / reject
options. Competitor presence alone is not a recommendation; every
option is stated so it stays compatible with the ruled read-authority
order (object -> user default -> server config -> public).

### #5837 — knowledge graph view cluster

Evidence: graph/backlink depth splits the field into three groups —
object/link-native (Anytype's graph over typed relations; Obsidian's
core graph + backlinks over file links; Logseq's linked/unlinked
references over blocks), backlinks-only (Notion, with backlinks
correctly scoped to what the viewer can access), and none documented
(Evernote, OneNote, Joplin natively). NotebookLM's Mind Maps are a
generated summary diagram over sources, not a link graph. The Open
Knowledge candidate (§3.9) would, if confirmed, add computed
backlinks over plain files.
Options: adopt — access-scoped backlinks as the floor (Notion's
scoping is the correctness bar: an edge to an object the resolved
policy denies must not leak its existence). Adapt — a graph view
whose edges inherit the resolved read decision per object, so the
public default never widens a restricted object. Reject — importing
Anytype-style typed-relation graphs wholesale before the content
model (#5839) settles what a link means in our model. Open: Drew's
graph-scope question (§6 Q4) bounds how far this generalises.

### #5838 — multi-device sync cluster

Evidence: two architectures. Local-first with user-chosen transport
(Obsidian plain files + any sync; Joplin's many targets with opt-in
E2EE; Logseq local graphs + optional RTC sync; Anytype's Any-Sync
with LAN peer-to-peer, self-hosted, and local-only modes) versus
vendor-cloud-only (Notion, Evernote v10+, OneNote on
OneDrive/SharePoint, NotebookLM). Failure modes differ accordingly:
local-first products document merge-on-reconnect; OneNote explicitly
forbids file-sync tools under its change-level sync.
Options: adopt — the local-first property that core use never
requires the vendor's server (four of eight products prove it is the
expected baseline in this category). Adapt — Anytype's mode menu
(network / LAN / self-hosted / local-only) as the shape of user
choice, with encryption on by default rather than Joplin's opt-in
model. Reject — a sync design that makes a single hosted service a
hard dependency for existing local content.

### #5839 — content model cluster (blocks vs pages vs daily notes)

Evidence: the field contains every point of the space — files
(Obsidian, Logseq file graphs), pages of blocks (Notion), block
outliner with journals (Logseq), typed objects (Anytype),
notebook/section/page hierarchy (OneNote), container-anchored notes
(Evernote note in exactly one notebook or space), and source
collections (NotebookLM). Two cautionary data points: Logseq is
mid-migration from files to a SQLite DB model and documents the
export loss that comes with it (Markdown export drops block
properties and timestamps); Notion's block model is the one whose
export cannot recreate the workspace.
Options: adopt — a normalized vocabulary (§2) as the comparison
contract for this cluster's own design work. Adapt — Logseq's
journal/daily-notes pattern as one entry surface over a page model,
not as the model itself. Reject — choosing a block-only or
object-only model on competitor prevalence; the matrix shows the
model choice is exactly where export fidelity is won or lost, so
#5839 should decide it together with #5844 and the export work in
issue #6439's line (not duplicated here).

### #5840 — sharing and permissions cluster (closed; evidence for the record)

Evidence: granularity ranges from page-level with five permission
levels and guests (Notion) through whole-container only (Anytype
spaces — members see everything in the space; OneNote notebooks,
with single-page sharing removed; Obsidian shared vaults with no
fine-grained permissions) to link-based single-note sharing
(Evernote public URL, Joplin public link). Anytype is the clearest
counter-example to per-object sharing: the vendor documents its
absence as the model.
Options (recorded, not a re-opening): the ruled order (object ->
user default -> server config -> public) is more expressive than
every model in the matrix except Notion's, and Notion's is the only
one with per-object grants to groups and users — adopt Notion's
guest pattern (grants to non-members for specific objects) as
compatible with the object step of the order; reject
container-all-or-nothing sharing (Anytype/Obsidian shape) as the
primary model, since it cannot express the order's object step.

### #5841 — capture and organisation cluster (closed; evidence for the record)

Evidence: a web clipper is universal among the capture-capable
products (Obsidian, Evernote, Notion, OneNote, Joplin, Anytype);
differentiation is in targeting and enrichment at capture time —
Notion clips into a database with properties, Anytype assigns a
Type and relations at clip time, Evernote chooses notebook and tags,
Joplin clips page variants including screenshots. NotebookLM
(collect sources) and Logseq's journal capture (share into today's
journal) are the two non-clipper shapes. Email-in exists but is
sometimes paywalled (Joplin Pro+).
Options (recorded): adopt clip-time classification (type/tags/target
chosen when capturing, per Notion/Anytype/Evernote) over
capture-now-sort-later; adapt Logseq's share-into-journal as the
mobile quick-capture floor; reject paywalling a capture channel
(email-in) in our own model — capture cost is not where the matrix
shows value being defended.

### #5842 — AI-over-notes cluster

Evidence: hosted vendor AI over the user's corpus is the commercial
mainstream (Evernote AI Assistant over the whole account; Notion AI
over the workspace plus connected apps; OneNote Copilot,
licence-gated; NotebookLM, where source-grounded AI is the entire
product; Joplin Cloud AI beta). The local-first products have no
built-in corpus AI: Obsidian documents AI only at clip time with a
bring-your-own model (including local models), and both Logseq and
Anytype expose the corpus to external AI through an MCP server /
local API instead of embedding a vendor model. Evernote's semantic
search indexes content, including attachment OCR text, on vendor
servers — the sharpest read-authority contrast in the matrix.
Options: adopt — the MCP/local-API exposure shape (Logseq, Anytype)
as the baseline: AI reaches exactly the objects the requesting
principal's resolved read decision allows, and the model choice
stays the user's. Adapt — NotebookLM's source-grounding discipline
(answers cite and stay within a selected source set) for any hosted
option. Reject — server-side indexing of the full corpus by default
(Evernote shape) for content whose resolved policy is not public: an
index is a read path, and it must resolve the same policy as any
other. Open: Drew's AI model question (§6 Q5) — local, hosted, or
user choice — decides which options are live.

### #5843 — extensibility cluster

Evidence: two families. In-app plugin systems (Obsidian core +
community plugins; Logseq plugin API and marketplace; Joplin
plugins that can even register import/export modules) versus
API-first (Notion's public API, connections, automations and
Workers; OneNote via Microsoft Graph, delegated-only; NotebookLM
enterprise-only API). The local-first products pair plugins with a
local API (Joplin's token-authenticated Data API; Anytype's local
HTTP API + MCP).
Options: adopt — a local, token-authenticated API over the corpus
(Joplin/Anytype shape) as the first extensibility surface: it serves
plugins, importers, and AI tools from one contract. Adapt — Joplin's
plugin-registered import/export modules for the importer line
(issues #6436, #6438). Reject — an enterprise-only API tier
(NotebookLM shape): it inverts this fleet's own operating model,
where the user's agents are first-class API consumers.

### #5844 — attachments and media cluster

Evidence: attachments are first-class resources in the local-first
products (Joplin resources, OCR-searchable, with size caps coming
from the sync plan rather than the app; Logseq assets under the
graph's assets/ directory with built-in PDF annotation; Obsidian
files in the vault folder) and embedded content in the cloud
products (Notion uploads — 5 MB free vs. 5 GB paid per file is the
steepest tier cliff in the matrix; OneNote file copies and printouts
that bloat notebooks; Evernote attachments counted against plan
caps). Export treatment diverges: Joplin JEX and Anytype Any-Block
carry attachments with fidelity; Evernote PDF export silently
excludes the actual attachment files (ENEX/HTML required).
Options: adopt — attachments as addressable resources with their own
identity (Joplin/Logseq shape), so the read-authority order applies
to an attachment as an object, not as a side effect of its parent
page. Adapt — PDF annotation (Logseq) as an attachment-native
feature rather than a viewer extra. Reject — export formats that
drop attachment payloads without saying so at export time
(Evernote PDF shape); any export we ship states what it excludes,
in the export UI, before it runs.

## 6. Open Drew-owned program questions (recorded, not answered)

From issues #5678 and #6435. These remain open; the matrix notes
where each affects rows. None is answered by assumption anywhere in
this document.

1. Product editions (free vs. paid) for evaluation scope — affects
   edition-sensitive cells: pricing/lock-in rows, AI availability
   (OneNote Copilot, Joplin Cloud AI), and sync limits (Obsidian
   Sync tiers, Evernote device caps).
2. Hands-on evaluation vs. documentation-only budget per product —
   this pass is documentation-only. Hands-on work could resolve the
   cells marked unknown where documentation is ambiguous, notably
   sync conflict behaviour, Evernote's current API status, OneNote
   and NotebookLM consumer export details, and Publish/PDF pricing
   gaps.
3. The identity of "Open Knowledge" — §3.9: unconfirmed. That
   product's matrix row is blocked until Drew identifies it; the
   Inkeep OpenKnowledge candidate's documented facts are recorded
   there, unverified as the intended referent.
4. Graph scope (all ID objects vs. knowledge-only in v1) — affects
   how far the #5837 finding generalises beyond notes.
5. AI-over-notes model choice (local, hosted, or user choice) and
   its read-authority implications — affects which #5842 options are
   live. Any model choice must consume only content the
   read-authority order resolves as readable for the requesting
   principal; a search index or AI context window is a read path.

## 7. Sources

All sources are vendor/primary unless marked (S) = secondary.
Observed 2026-11-08.

- Obsidian: obsidian.md/help (vault, sync/collaborate, import,
  web-clipper, plugins, Publish media files); obsidian.md pricing
  and Sync pages; docs.obsidian.md plugin submission.
- Evernote: help.evernote.com articles (export as ENEX/HTML, export
  as PDF, import, AI Features FAQ, AI Assistant, Starter/Advanced
  plan FAQ, local notebooks); evernote.com/blog (spaces/stacks,
  Web Clipper, v11); dev.evernote.com legacy sharing docs;
  evernote.com/learn offline guide.
- Notion: notion.com/help (export your content, import data,
  sharing settings guide, create links and backlinks, use pages
  offline, web clipper, images/files/media, Notion AI guide,
  integrations/API); notion.com/pricing; developers.notion.com
  files and media guide.
- OneNote: support.microsoft.com (share a notebook, sync a notebook,
  download/export/import notebooks, Copilot in OneNote, insert or
  attach files, Web Clipper getting started);
  learn.microsoft.com Graph OneNote API overview.
- NotebookLM / Gemini Notebook: blog.google (NotebookLM becomes
  Gemini Notebook, 2026-07-16); support.google.com/notebooklm (Mind
  Maps); cloud.google.com Gemini Enterprise / NotebookLM Enterprise
  docs (share notebooks, API notebooks and sources);
  edu.google.com NotebookLM page.
- Logseq: github.com/logseq/logseq README;
  github.com/logseq/docs db-version.md (nodes, sync/RTC, publish,
  export and import, assets, MCP server, plugins, iOS app);
  discuss.logseq.com (database version statement).
- Joplin: joplinapp.org/help (sync, terminal, REST API reference);
  joplinapp.org/plans.
- Anytype: blog.anytype.io (a new metaphor);
  business.anytype.io/privacy; Google Play listing; Chrome Web Store
  (Anytype Web Clipper); pricing figures via (S) koru.do and
  blockchainreporter.net reporting anytype.io/pricing, checked
  2026-10-19 and 2026-11-04.
- Open Knowledge candidates: openknowledge.ai/docs (core concepts,
  CLI reference, editor features); ssdnodes.com/learn (Google Cloud
  OKF); okfn.org (Open Knowledge Foundation).
