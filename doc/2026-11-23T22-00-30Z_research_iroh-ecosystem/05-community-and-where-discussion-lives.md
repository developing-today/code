# Community and where discussion lives

*Part of the [iroh ecosystem research record](2026-11-23T22-00-30Z_research_iroh-ecosystem.md).
Everything from the community below is paraphrased in substance —
follow the links to read the originals.*

## The two main venues, plus satellites

The "discussion board somewhere, not Zulip, maybe Slack" recollection
resolves to two main venues:

1. **The Iroh Discord — the primary social venue.** This is the
   Slack-like place. n0's own materials route people there constantly:
   the iroh README carries a Discord badge
   ([invite](https://discord.com/invite/DpmJgtU7cW)), the contributing
   guide sends people there for mentoring through bugs and features
   ([CONTRIBUTING.md](https://raw.githubusercontent.com/n0-computer/iroh/main/CONTRIBUTING.md)),
   and the canonical short link is **https://iroh.computer/discord**.
   **Access reality:** Discord requires an account and joining the
   server; the channels are *not publicly readable* from the open
   web. This research did not join or read it — nothing behind that
   login is summarised here. Joining is a step only the account holder
   can take.
2. **GitHub Discussions on n0-computer/iroh — the public board.**
   [github.com/n0-computer/iroh/discussions](https://github.com/n0-computer/iroh/discussions)
   is fully world-readable without an account, with categories
   **Announcements, Q&A, Ideas, Show and tell, General**. Release
   announcements are posted here by the maintainers (the 1.0.x patch
   releases in July 2026, 1.1.0 in August, 1.3.0 in September, 1.4.0
   and 1.4.1 in October 2026 — the fast post-1.0 cadence), and the
   contributing guide names Discussions as the place for concept
   questions and help. Volume is modest — this is the considered,
   on-the-record layer, not the firehose.
3. **Satellites:** a semi-official Matrix room and a Mastodon account
   (@n0iroh) are linked from
   [awesome-iroh](https://github.com/n0-computer/awesome-iroh)'s
   Follow section, alongside the official blog, @iroh_n0 on X, and
   the number 0 YouTube channel. Announcements flow through the blog +
   Discussions + socials; day-to-day developer socialising is
   Discord.

**Bottom line:** the substantive *public* record is GitHub
Discussions + the blog; the substantive *social* record is the
login-gated Discord.

## Recent important conversations (public board, paraphrased)

- **A feature is being worked on that will have a major impact:
  post-quantum endpoint identity is now an open design
  conversation.** A September 2026 Ideas thread argues that because
  endpoint IDs are Ed25519 keys, a future fault-tolerant quantum
  computer could forge identities, and that migration across a
  deployed identity system takes years — so the proposal is to make
  iroh crypto-agile and define a post-quantum migration path *now*
  while keeping Ed25519 today. The thread drew sustained maintainer
  engagement.
  [Discussion #4532](https://github.com/n0-computer/iroh/discussions/4532)
- **A decision was made about how WebRTC support will happen: as an
  external transport crate, not an in-tree feature.** A March 2026
  General thread started by a maintainer lays out that the team wants
  WebRTC as a custom transport (the API for custom transports landed
  in v0.97) but could not prioritise building it before roughly
  mid-2026; maintainers then steered the work to a standalone crate
  built on the str0m WebRTC implementation, following the pattern of
  the existing Tor and Nym transport crates. A community contributor
  has since reported a working implementation covering
  native-to-native, browser-to-browser and browser-to-native
  connections.
  [Discussion #4024](https://github.com/n0-computer/iroh/discussions/4024)
- **Someone showed a project: FlipFlop** — persistent tunnels between
  peers for sharing files, text and songs and running microapps,
  posted to Show and tell on 2026-11-21 — the freshest community
  project on the board at research time.
  [Discussions board, Show and tell](https://github.com/n0-computer/iroh/discussions)
- **This was recently discussed and is an open architectural
  question: using relays for coordination only, never for application
  data** (Ideas, November 2026) — and a sibling July 2026 thread asks
  for the related ability to restrict n0's relays to hole-punching
  only, with subsequent traffic barred from the relay. Relay-traffic
  minimisation is a live design pressure from the community.
  [Discussions board, Ideas](https://github.com/n0-computer/iroh/discussions)
- **Someone showed a project: Sqwakie Direct** (Show and tell,
  October 2026) — a signed team log for AI agents communicating over
  iroh QUIC; and **Tincan** (October 2026), peer-to-peer voice and
  text chat in the terminal.
  [Discussions board, Show and tell](https://github.com/n0-computer/iroh/discussions)
- **Hardware-backed endpoint identity is being explored**: an April
  2026 Ideas thread seeks architectural guidance on keeping endpoint
  secret keys in TPM 2.0 / Secure Enclave hardware, with maintainers
  and the post-quantum proposer participating — identity-key custody
  is an active community concern, parallel to this estate's CA work.
  [Discussions board, Ideas](https://github.com/n0-computer/iroh/discussions)
- **Group discovery is an unsolved want**: an October 2026 Ideas
  thread asks how to find the endpoint IDs of nodes belonging to a
  "group" — there is no built-in group-membership discovery
  primitive, which is precisely the layer this estate's auth
  investigation is about.
  [Discussions board, Ideas](https://github.com/n0-computer/iroh/discussions)
- **Authorisation approaches remain an open question on the board**:
  the long-running "Approaches for adding authorization?" General
  thread (started February 2025) is the board's main public record of
  users working out how to layer authorisation over iroh's
  authenticate-by-key transport — iroh itself ships no authorisation
  model.
  [Discussions board, General](https://github.com/n0-computer/iroh/discussions)

## What is *not* publicly visible

The Discord's content — help channels, working discussion, any
decision-making that happens socially — is not readable without
joining, and this research deliberately did not join under anyone's
identity. The public surfaces above (Discussions, the blog, release
notes, awesome-iroh) are a faithful record of *what shipped and what
was proposed*, but the community's live pulse is in the Discord at
https://iroh.computer/discord, and joining it is a one-click step
from one's own account.
