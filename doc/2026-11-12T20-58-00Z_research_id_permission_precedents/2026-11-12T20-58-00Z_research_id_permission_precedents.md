# Research: Permission-prompt precedents — how platforms present, remember, and revoke capability grants

- Date: 2026-11-12
- Status: **research record** for the permission-UX design; it surveys prior art and maps it onto the design's adopted patterns. It changes no design decision by itself.
- Issue: [#5961](https://github.com/developing-today/code/issues/5961), part of the worlds design programme [#5955](https://github.com/developing-today/code/issues/5955) (statements S19–S23 of the 2026-11-04 brief).
- Consumer: the permission-UX design [#5960](https://github.com/developing-today/code/issues/5960), landed as PR [#7594](https://github.com/developing-today/code/pull/7594) in `doc/2026-11-08T09-06-08Z_design_id_permission_ux/`, which names this survey as its unresolved precedent dependency.
- Method: every platform claim below is drawn from a primary source (vendor developer documentation, the OAuth RFCs, or the cited study) fetched and read on 2026-11-12. Facts that surfaced in secondary sources but could not be verified against a fetched primary page are marked **[unverified in this pass]** and are not load-bearing for the shortlist.

## 1. Per-system survey

### 1.1 Google Play — listing-time disclosure, developer-side declarations

- **Shown:** Two pre-install disclosures on the store listing: (a) the **Data safety section** — developer-declared data types collected/shared (a fixed taxonomy: location, personal info, financial, health, messages, photos, contacts, …), purposes, whether collection is optional, encryption in transit, and data-deletion paths; and (b) an **app permissions list** of capabilities the app can or might request. Separately, developer-side: apps requesting high-risk/sensitive permissions (SMS, call log, background location, …) must file a **Permissions declaration form** in Play Console (core-functionality justification, review instructions, video demo) and be approved before publishing.
- **When:** Disclosure is **pre-install**, on the listing. The declaration gate is **pre-publication**. Play itself grants nothing — user grants happen later via Android runtime prompts (§1.2).
- **Remembered:** Nothing user-grant-like is stored by Play; listings show the currently declared practices across distributed versions. A new permission in an update requires a revised declaration.
- **Revoked/expired:** There is no Play-level grant to revoke; the Data safety surface routes users to Android's per-app permission controls. Non-compliant declarations block updates or remove the app.
- Sources: [Data safety section](https://support.google.com/googleplay/answer/11416267); [Permissions declaration requirements](https://support.google.com/googleplay/android-developer/answer/9214102).

### 1.2 Android runtime permissions — just-in-time, per-permission memory

- **Shown:** A system dialog the app cannot customize, naming the permission (group) requested (location, camera, microphone, …). Google's guidance is that the app explain "why" in its own in-context UI before triggering the dialog. "Dangerous" permissions are granular; normal permissions are granted automatically.
- **When:** **Just-in-time, in context** — official guidance: ask when the user starts to interact with the feature that requires the permission. Flow: declare in manifest → user invokes the feature → check → optional rationale UI → system prompt.
- **Remembered:** Grants persist per app + permission until changed. Since Android 11, location/camera/microphone dialogs offer **"Only this time"** — a one-time grant lasting while the activity is visible (and briefly after), with a re-prompt next use. Denying twice becomes a permanent denial ("don't ask again"); the dialog stops appearing.
- **Revoked/expired:** Users can revoke any permission in system settings (revoking kills the app process). Android **auto-resets permissions of unused apps** back to denied, and apps on Android 13+ can programmatically relinquish permissions they no longer need.
- Source: [Request runtime permissions](https://developer.android.com/training/permissions/requesting).

### 1.3 iOS — privacy labels up front, purpose strings at the prompt

- **Shown:** (a) **Privacy Nutrition Labels** on the App Store product page, before download: data types collected, whether linked to the user's identity, and whether used to track — developer-supplied in App Store Connect, fed by privacy manifests from the app and its SDKs. (b) The **App Tracking Transparency (ATT)** system prompt for tracking/IDFA access, which must carry a developer-supplied **purpose string**; without a grant the advertising identifier is all zeros. (c) Per-resource system alerts for protected data classes, likewise carrying purpose strings.
- **When:** Labels are **pre-install**. ATT is presented when the developer chooses, but the identifier is only returned after the prompt is shown and granted — effectively at first tracking use.
- **Remembered:** The ATT choice is stored per app; a global "Allow Apps to Request to Track" setting can suppress prompts entirely.
- **Revoked/expired:** Users change tracking permission per app in Settings; turning tracking off per-app or globally revokes it.
- Source: [User privacy and data use](https://developer.apple.com/app-store/user-privacy-and-data-use/).
- **[unverified in this pass]** Fine-grained iOS runtime-prompt rules beyond ATT and the labels (per-resource prompt timing, the Settings > Privacy & Security per-category lists) — the HIG Privacy and ATT framework pages failed to load when this survey was compiled and were not re-fetched; the shortlist does not depend on them.

### 1.4 Chrome extensions — install-time warnings, runtime optional permissions

- **Shown:** At install, a list of **permission warnings** in fixed plain language (e.g. "Read and change all your data on the websites you visit" for broad host access); each API/host permission maps to a fixed warning string, and some permissions show no warning. Runtime requests for optional permissions show a confirmation prompt with the new warnings. Granularity: named API permissions plus host permissions as per-site match patterns, declared as required (`permissions`/`host_permissions`) vs optional (`optional_permissions`/`optional_host_permissions`).
- **When:** Required permissions are consented **at install**; optional permissions are requested **at runtime, from inside a user gesture**, when the relevant feature is enabled. An update that adds required permissions re-triggers warnings; one that adds only optional permissions does not disable the extension.
- **Remembered:** Granted optional permissions persist per extension until removed; `chrome.permissions.getAll()/contains()` expose current grants, with `onAdded`/`onRemoved` events so the extension can react.
- **Revoked/expired:** Extensions can self-revoke via `chrome.permissions.remove()` (after removal, re-requesting usually re-adds without a prompt). Users manage per-extension toggles on the extension details page.
- Sources: [Declare permissions](https://developer.chrome.com/docs/extensions/develop/concepts/declare-permissions); [chrome.permissions API](https://developer.chrome.com/docs/extensions/mv2/reference/permissions).
- **[unverified in this pass]** `activeTab`'s temporary-grant semantics and the user-facing per-site "site access" control (search-surfaced, pages not fetched).

### 1.5 Firefox extensions — human-readable install prompt, updatable grants

- **Shown:** An install prompt listing human-readable permission messages ("Access your data for all websites", "Access your data for sites in the [named] domain", "Read and modify bookmarks", "Access browsing history", …); only a defined subset of permissions produces messages — `activeTab`, `storage`, `cookies`, `alarms` and others show none. Recent Firefox versions also show the manifest's declared **data-collection** types in the install prompt alongside permissions.
- **When:** Required permissions at **install**. If an **update** needs new permissions, the user must approve them first — cancelling keeps the previous version. Optional permissions are requested **at runtime** via `permissions.request()`, only inside a user-action handler; a multi-permission request is all-or-nothing.
- **Remembered:** Grants persist per extension; runtime grants can be granted or revoked by the user at any time in the add-on's "Permissions and data" settings (about:addons), and extensions are expected to listen for `permissions.onAdded`/`onRemoved`.
- **Revoked/expired:** Via about:addons toggles, programmatically via the Permissions API, or by removing the extension; declining an update's new permissions blocks that update only.
- Source: [Request the right permissions](https://extensionworkshop.com/documentation/develop/request-the-right-permissions/).

### 1.6 OAuth 2.0 — scoped tokens, server-side grants, admin pre-consent

- **Shown:** The spec standardizes machinery, not UI: a client requests a `scope` (space-delimited strings, RFC 6749 §3.3) at the authorization endpoint, and the authorization server obtains the resource owner's approval before issuing a token "denoting a specific scope, lifetime, and other access attributes"; the granted scope may differ from the requested scope and is reported back. In Google's implementation the consent screen lists the app and the service APIs/scopes requested.
- **When:** At **first authorization** (redirect when access is first needed), and again if new scopes are requested — **incremental authorization** is the norm. Admin pre-consent exists in Google Workspace: admins mark apps **Trusted** (all scopes, including restricted), **Limited**, or **Blocked** in Admin console API controls, and approved Marketplace apps can be domain-installed, **bypassing the user consent screen**.
- **Remembered:** The grant is stored server-side per user + client as issued access/refresh tokens; access tokens carry an expiry (`expires_in`); refresh tokens persist until revoked. Admin consoles list configured/accessed apps with their scopes and user counts.
- **Revoked/expired:** **RFC 7009** defines a revocation endpoint: POSTing a token invalidates it and, per server policy, other tokens from the same grant and the grant itself (refresh-token revocation MUST be supported). Resource owners can revoke grants at any time outside the protocol; Workspace admins can revoke an app's access, forcing re-authorization.
- Sources: [RFC 6749](https://datatracker.ietf.org/doc/html/rfc6749); [RFC 7009](https://datatracker.ietf.org/doc/html/rfc7009); [Google Workspace API controls](https://support.google.com/a/answer/13152743).

### 1.7 macOS TCC — per-app, per-category remembered consent

- **Shown:** System prompts of the form "App would like to access…" for protected resources (Documents/Downloads/Desktop/iCloud Drive files, camera, microphone, …). Some categories are **not promptable in-app**: Full Disk Access must be added explicitly by the user, and Accessibility/Automation require user changes, in **System Settings > Privacy & Security**.
- **When:** **At first access attempt** for promptable categories; settings-only categories are effectively user-initiated, out-of-band grants — no in-app prompt can grant them.
- **Remembered:** Decisions persist per app per category; the Privacy & Security lists *are* the record — an app appears in a list after it has requested that access.
- **Revoked/expired:** Users review and toggle off any grant in System Settings > Privacy & Security at any time.
- Source: [Controlling app access to files (Apple Platform Security)](https://support.apple.com/guide/security/controlling-app-access-to-files-secddd1d86a6/web).
- **[unverified in this pass]** TCC internals from secondary sources (the `tccd` daemon, TCC.db stores, `tccutil reset`, MDM/PPPC pre-approval rules) were not confirmed against a fetched Apple page; the consent-model claims above are what the fetched page supports.

### 1.8 Flatpak portals — consent implicit in the act of use

- **Shown:** Two layers. (a) **Static** sandbox permissions declared in the app manifest `finish-args` (`--filesystem`, `--device`, `--socket`, `--share`, …) — set at build time, visible via store/tooling, never prompted. (b) **Dynamic** portal interactions at runtime: for many resources, consent is *implicit in a system UI action* — picking a file in the native FileChooser grants access to exactly that file; device portals (e.g. USB) show an allow/deny dialog for the specific device. Portal models can be very fine-grained: the USB portal distinguishes a blanket per-app permission from a per-device permission "down to the serial number".
- **When:** Portals mediate **at the moment of use** — the app calls the portal when it needs the resource; the portal requests permission if not already granted and hands back a file descriptor on grant.
- **Remembered:** Dynamic portal decisions are stored per app in the **permission store** (`org.freedesktop.impl.portal.PermissionStore`); static grants persist as manifest/override configuration.
- **Revoked/expired:** Portal-issued file descriptors can be revoked by the portal at any time, and apps release access explicitly (e.g. `ReleaseDevices()`); static permissions change via overrides/manifest.
- Sources: [Sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html); [USB portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Usb.html).
- **[unverified in this pass]** The end-user management workflow (`flatpak permission-list`/`permission-show`/`permission-reset`, Flatseal) — search-surfaced only; the fetched docs verify the permission store and per-app storage, not the CLI surface.

## 2. What the failure literature says

- **Install-time-only lists did not inform anyone.** Felt et al. (SOUPS 2012) ran a 308-user survey and a 25-user lab study of install-time Android permissions: **17%** of participants paid attention to permissions during a given installation, **42%** of lab participants were unaware permissions existed at all, and only **3%** of survey respondents answered all comprehension questions correctly. Permissions shown once, up front, in a list, failed at attention, comprehension, and behaviour — the direct evidence behind Android's move to runtime prompts in Android 6.0. ([paper](https://people.eecs.berkeley.edu/~daw/papers/anduser-soups12.pdf))
- **Read ≠ understood.** Kelley et al. (2012) found permission displays are generally viewed and read but not understood, and that users widely believed the marketplace itself vetted apps — i.e. a store/listing disclosure does not transfer responsibility to the user in the user's own model. ([A Conundrum of Permissions](https://www.researchgate.net/publication/255564201_A_Conundrum_of_Permissions_Installing_Applications_on_an_Android_Smartphone))
- **Prompt fatigue and blanket acceptance.** The consistent pattern across the systems above: prompts that arrive out of context, in bundles, or too often train blanket acceptance (OAuth's bundled-scope consent screens are the canonical complaint — apps request maximal scopes up front because re-prompting is costly, users accept because the alternative is not using the service). Every platform that started with install-time bundles (Android, Chrome) later added a runtime/incremental path; none went the other way.

## 3. Comparison table

| System | What's shown | When | What's remembered | Revocation / expiry |
|---|---|---|---|---|
| Google Play | Data safety section + permissions list (developer-declared) | Pre-install (listing); declarations pre-publication | Declared practices only; no user grant | N/A at Play level; routes to Android settings |
| Android runtime | System dialog naming the permission group; app-supplied rationale UI before it | Just-in-time, when the feature is invoked | Per app + permission; "Only this time" one-shot option; double-deny = permanent | Settings per permission; auto-reset for unused apps; apps can self-relinquish (13+) |
| iOS | Privacy labels (store page); ATT prompt with purpose string; per-resource alerts | Labels pre-install; ATT at developer-chosen point before tracking | Per app; global tracking-requests switch suppresses prompts | Settings per app / global switch |
| Chrome ext. | Fixed plain-language warnings per permission; host patterns per site | Required at install; optional at runtime inside a user gesture | Per extension; grants queryable via API with change events | `chrome.permissions.remove()`; details-page toggles |
| Firefox ext. | Human-readable messages for a defined permission subset; data-collection types in prompt | Required at install; update additions need approval or update is blocked; optional at runtime in a user-action handler | Per extension; user-editable in about:addons | about:addons toggles; Permissions API; declining blocks only the update |
| OAuth 2.0 | Consent screen listing app + scopes (implementation-defined) | First authorization; again for new scopes (incremental) | Server-side grant per user + client; expiring access tokens + persistent refresh tokens | RFC 7009 revocation endpoint; owner/admin revocation at any time |
| macOS TCC | "App would like to access…" system prompts; some categories settings-only | First access attempt; settings-only categories are user-initiated | Per app per category; the Settings lists are the record | Toggle off in Privacy & Security at any time |
| Flatpak portals | Static manifest args (never prompted); portal dialogs, or consent implicit in a native picker action | At the moment of use | Per app in the portal permission store; static args as configuration | Portal can revoke FDs anytime; overrides change static grants |

## 4. Shortlist for the two-sided (client + server-admin) grant model

Patterns are stated with the precedent that supports them and how they land in the permission-UX design ([#5960](https://github.com/developing-today/code/issues/5960), PR #7594). Nothing here reopens a settled design decision; §5 records the mapping and the two places where the survey adds information the design did not have.

**Adopt — disclosure before connection (listing-time layer).** Play's Data safety section and iOS labels show the shape: a fixed-taxonomy, plain-language declaration shown *before* commitment, separate from any grant. The design's pre-connect disclosure is exactly this layer, with one improvement the precedents lack: splitting the list into client-side and server-side groups, because Play/iOS lists address a single party. Caveat from §2: this layer informs; it must not be treated as consent (Kelley: users read the store page as the store's vetting, not their own decision).

**Adopt — in-context prompts, at the design's settled timing.** Android runtime prompts, TCC first-access prompts, Chrome/Firefox optional-permission requests, and portal mediation all converged on asking in context, attached to a concrete decision rather than a settings page. The design's settled timing is *not* per-capability first use: its §1.2 requires the client-side grant decision at **first connect**, before any capability is used (with pre-connect disclosure explicitly not treated as consent), and in-context prompting thereafter only for capability changes, showing the delta. What the precedents support is that shape — per-capability lines decided in context with the counterparty named — plus the design's graceful-degradation rule: a capability line denied at connect leaves the world loaded with that capability inert, the same shape as an Android app running with a denied permission. Per-capability just-in-time prompting (ask when each capability is first exercised) diverges from the settled first-connect timing and is recorded as a proposed divergence in §5, not adopted here.

**Adopt — per-scope remembered grants with a settings surface.** Every system stores decisions per (principal, capability): Android per app + permission, TCC per app + category, browsers per extension, OAuth per user + client, portals per app. All expose a review/revoke surface (Settings lists, about:addons, admin consoles, the portal permission store). The design's memory model (per world, per permission, or global) matches; the precedent adds one requirement the design already implies: the memory must be *inspectable and editable in one place*, because that is where revocation lives in every precedent.

**Adopt — admin pre-consent for the server side, disclosed-not-asked for users.** Google Workspace's Trusted/Limited/Blocked app lists and domain install are the direct precedent for the design's asymmetry: server-side capabilities are an operator decision (pre-consented, blocked, or limited by policy), and the connecting user is shown them as disclosure only — the prompt informs, it does not ask the user to approve someone else's machine. TCC's settings-only categories (Full Disk Access cannot be prompted in-app) are the client-side analog: some grants exist only as deliberate out-of-band configuration, never as an in-flow prompt.

**Adopt — incremental requests over bundles.** OAuth incremental authorization and Chrome/Firefox optional permissions exist because install-time bundles produced scope creep and blanket acceptance (§2). A world's requested set should be allowed to grow at runtime — with a fresh, in-context prompt — rather than being front-loaded at connect; the design's capability-change notices (sibling [#5959](https://github.com/developing-today/code/issues/5959)) are the disclosure half of this pattern.

**Adopt — one-time / session-scoped grants as a first-class mode.** Android's "Only this time" (and portal FD hand-outs that live only until released) are the precedents for a grant that expires by default. See §5: the design's grant-mode matrix does not currently name this mode.

**Reject — install-time-only, all-or-nothing consent.** The Felt et al. numbers (§2) and both mobile platforms' migration away from it are decisive; also rejected implicitly by every extension platform adding runtime requests. The design already rejects this; the survey is its citation.

**Reject — warnings as free text.** Chrome's fixed warning strings and Play's fixed data taxonomy exist because developer-written descriptions drift toward reassurance. Capability descriptions in the disclosure should come from a fixed registry of plain-language strings keyed to the capability, not from the world's manifest free text (the manifest names *which* capabilities; the platform owns *how they are described*). iOS purpose strings are the partial counter-example — developer-supplied, but attached to a system prompt the developer cannot otherwise customize.

## 5. Effect on the permission-UX design (#5960)

The survey **confirms** the design's core moves, with citations now available per decision: Play-shaped pre-connect disclosure (§1.1, §1.3), two-party split (no precedent merges two parties' capabilities into one list; Workspace pre-consent, §1.6, is the closest two-sided model), per-capability client granting with memory (§1.2, §1.7), admin-decided server side (§1.6), and load-bearing-vs-optional degradation (Android's denied-permission app behaviour, §1.2).

Two observations the design record did not previously have, offered as survey output rather than decision changes:

1. **A one-time/session grant mode.** Android ships "Only this time" as a peer of allow/deny (§1.2); the design's grant-mode matrix (approve generally / ignore for all time / ignore for specific permissions / YOLO) has no expiring-grant entry. If the design owner adopts it, this survey is the citation; if not, the absence is now a recorded choice.
2. **Grant decay for unused worlds.** Android's auto-reset of unused apps' permissions (§1.2) is the only precedent for time-based grant hygiene. The design's memory is indefinite; whether a world not visited for N months should fall back to ask-again is an open question this survey raises and deliberately does not settle.
3. **Proposed divergence — per-capability just-in-time prompting.** The dominant precedent timing (Android, TCC, extension optional permissions, portals) asks when a capability is first exercised. The design's settled §1.2 timing is a decision at first connect, before capability use, with delta prompts on capability change. Moving client-side grants to per-capability first-use prompts would change that settled contract and requires a design decision by the #5960 owner; this survey records the divergence and takes no position on reopening it.

## 6. Sources

Primary sources fetched and read on 2026-11-12 (platform sections cite them inline):

- Google Play Data safety: https://support.google.com/googleplay/answer/11416267
- Google Play permissions declarations: https://support.google.com/googleplay/android-developer/answer/9214102
- Android runtime permissions: https://developer.android.com/training/permissions/requesting
- Apple user privacy and data use: https://developer.apple.com/app-store/user-privacy-and-data-use/
- Chrome declare permissions: https://developer.chrome.com/docs/extensions/develop/concepts/declare-permissions
- Chrome permissions API: https://developer.chrome.com/docs/extensions/mv2/reference/permissions
- Firefox request-the-right-permissions: https://extensionworkshop.com/documentation/develop/request-the-right-permissions/
- RFC 6749 (OAuth 2.0): https://datatracker.ietf.org/doc/html/rfc6749
- RFC 7009 (token revocation): https://datatracker.ietf.org/doc/html/rfc7009
- Google Workspace API controls: https://support.google.com/a/answer/13152743
- Apple Platform Security, app access to files: https://support.apple.com/guide/security/controlling-app-access-to-files-secddd1d86a6/web
- Flatpak sandbox permissions: https://docs.flatpak.org/en/latest/sandbox-permissions.html
- xdg-desktop-portal USB portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Usb.html
- Felt et al., "Android Permissions: User Attention, Comprehension, and Behavior", SOUPS 2012: https://people.eecs.berkeley.edu/~daw/papers/anduser-soups12.pdf
- Kelley et al., "A Conundrum of Permissions", 2012: https://www.researchgate.net/publication/255564201_A_Conundrum_of_Permissions_Installing_Applications_on_an_Android_Smartphone

Verification gaps (flagged inline where relevant): the Apple HIG Privacy and ATT framework pages and the Chrome Web Store help page did not load during compilation; claims depending only on them are marked **[unverified in this pass]** and none are load-bearing for §4–§5.
