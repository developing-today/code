# Brief: the Nix cache as the content store — derivations for everything

Drew, 2026-11-13 05:42 CST. Drew flagged this himself as "kind of separate" from the pool/effort program; it is filed as its own design program. Transcribed faithfully and sectioned.

## 1. The core question

Explore whether **our Nix cache could be the content store directly**: all files, sets of files, etc., converted into derivations of the content-addressed store. Ideally one day one of the Nixes supports all the performance and flexibility of recursive Nix and Nix trees. Also in scope: the concept of **trustless Nix** (there was a "Trust Nix" or something) and all the tools required, plus optional features to support it.

## 2. Flakes per item

- **Various flakes for every single item**, perhaps sharded by the content-addressed prefix or some other method — maybe one flake per derivation — to reduce flake resolution time end-to-end and prevent or partially mitigate flake-lock conflicts.
- Support standard-library references to Nix packages using whatever the core flake uses, following it or pinning a specific one. Individual use cases can override to use the latest. Add **version constraints**, which Datadog has added either into the flake or as another file.

## 3. Version scheme

Natively support sending versions, automatically translating all changes into the **ISO date.increment** for the given day. With forks:

- the version of the original: `.1`
- the first fork: `.1`
- the first change of the first fork: (continues the pattern)

Start at 1, so the first fork could be `.1` — plus any other appropriate tags and signifiers for a given item. (The exact suffix algebra is for the design to pin down; the brief records the intent: date-based versions, fork lineage visible in the version.)

## 4. Resolution semantics

- Flake inputs that don't carry a forbidding version constraint resolve to the **newest version allowed by every constraint**, following single versions from the root down through all the files.
- Sometimes a version for one eval could be slightly newer for a file used in 50 other places: we can **downgrade** the newer file (from the flake lock) for that one derivation if the constraints allow it.
- We can **allow divergence**: one derivation keeps its original version while all the rest use another — a conscious, documented choice to allow two versions of the same essential derivation for a pragmatic reason, recorded where we decided it.

## 5. The benchmark mandate

Making a derivation + a flake per item and resolving the full chain may be too high a cost. **Benchmark it and determine exactly why**, either way:

- Maybe it uses too much space.
- Maybe it's very efficient — techniques to automatically handle storage costs, compression, and storage tiering.
- Maybe storage is a wash, but three extra files + a `nix eval` per derivation is too much overhead.
- Or those are fine, but generating a complex flake of hundreds of objects doesn't work.

Decision rule: if it has no significant downsides — essentially a wash — **keep it**, because it's not hurting anything; we can rip it out later, and we will likely find good uses when building more adaptable interfaces.

Benchmark and evaluate features, errors, and bugs across **every major Nix compiler, Nix store backend, and resolver** (Nix, Lix, Tvix, Trust Nix, Hydra's learnings, nixpkgs practice). One released combination ideally represents the best combination for each aspect — but also explore **mixed pairings**: compiler A with resolver and store B; compiler B + resolver B with store B as a hot cache falling back to a long-term storage implementation of store A for its storage characteristics and feature support. If all compilers can evaluate the expression, they should all build it or error the same way — test whether that's true, and map where pairings are compatible one way or the other. Value may exist in: one toolset for *building* a derivation uploaded to one or more stores (serially or in parallel), a different combination for *fastest resolution* of the full dependency tree, and another for providing the full already-built derivation for a single dependency (`nix eval` without needing the full chain files).

## 6. Pruning and module surfaces

- Files used to build binaries that don't land in the final target probably aren't required, or can be copied into the final location if required. Prune everything except the final binary (or the final binary + some config files), depending on the consumer: a flake app, a NixOS program, a home-manager service or system packages, a home-manager app for a specific user or all users, an overlay program, a systemd or launchd unit, and so on.
- Perhaps a **home-manager module and a NixOS module** enable required settings and expose **conflicts** between modules used at the same time.

## 7. The target matrix

The configuration may be a wash for some purposes but a benefit when generating:

- a binary for **Darwin, Ubuntu, NixOS, or Red Hat**; a **VM or buildkit container** from scratch or from a base image
- scaling/portability: executing builds or many instances of a program simultaneously; dynamically auto-scaling work in each container/VM, balancing across clusters and latency; spilling into a version built for **workers, a VPS, or cloud compute**; **Cloudflare containers or workers running on containers**
- an ideal endpoint for a from-scratch container using an exposed **static binary**; a **dynamic** binary option
- the resultant derivation **with or without its dependencies**, as a library or dependency of another derivation, automatically offered in all versions: VM from scratch · static · dynamic · **Ubuntu package · RPM package · Arch / Omarchy packaging · snap · Homebrew tap or package** · **Windows installer** (SIS or the modern equivalent) · raw files · zip / tar / tar.gz / tar.zstd / zstd streams
- a version **embeddable in a Rust or Go program** as a library or DLL (pure Go, or C++/Go)

### Roc + ID integration

- A **roc** library/version embeds **ID** and essentially boots a blank ID server directly into the given world. Wrapped and unwrapped variants: unwrapped includes all build files and Nix files (doesn't make it into the target); wrapped variants carry an appropriate ID store, iroh identity, and config around the blank builds.
- Dynamic builds can share one or a small set of ID generation versions; each ID derivation object launches as a standalone executable or script (**roc as the shebang shell interpreter**), with platform controls used inside and outside Nix depending on context.
- For all worlds / standalone functions / ID objects / groups of files executed using roc or exposed as a roc library (in addition to or instead of an ID library, standalone executable, or world/program), support **roc tunics and ID tunics**, and a variant that **traces all roc and ID dependencies into a single file** which compiles without duplicating the same file in 50 places and without relying on downloading Blake3-hashed files/values from the ID store (known at compile time; gathered via network + hash, integrated and deduced). Cases left to runtime scripts: gathering a dependency, generating a config file, downloading a file from a specific URL or ID namespace — only for setting the current user, gathering today's article of the day, or setting current defaults and dynamic content the world needs, following the IFRID.
- Wrappers around accessing ID / storing and retrieving from ID stores set up all that's known to be needed; a generic interface pulling many ID objects and roc code/libraries and running its own compiles (dynamically, based on inputs and purpose). Those dynamic calls may reach through an endpoint or an ID server backed by a Nix store — even the **same Nix store, live**, instead of during compilation. Cache final outputs, all Nix code and files, dependency resolutions, and references to build an output. Break things apart so long-compiling parts only run when needed and don't get polluted when unrelated files change.
- Roc just needs a platform, which could have any number of compilation steps. Investigate how best to use Roc→Nix, and when to build DLLs or wrappers that compile from Roc but are called as DLLs through the platform.

### Web / Wasm / app variants

- Support **Wasm in the browser, WASI** on Unix/Mac/Windows, different interfaces per environment:
  - web-served version embedding/deploying as a web server on 8080 or 3000
  - version preparing files for **nginx or Caddy**
  - website version running **ID serve web**, outputting the web server directly from REST code
  - version generating the ID variant for **Cloudflare Workers, D1, R2, Turso**, supporting IndexedDB
- A wrapper that builds a world as an **Android APK** by flipping the same kind of flag used to build a standalone web-server binary; a flag to generate a compressed Blake3-hashed file published as a roc dependency, fully standalone; a version that runs as a terminal program or exposes a **ratatui** interface, with or without remote SSH; a version generating WASI/WASM for the browser, usable as a library or to generate files other web servers serve or embed. The web WASM can be called by local ID binaries, local ID JavaScript executables, ID web servers, Cloudflare Workers, and Durable Objects — for that last one: provide the WASM to the client, host a wrapper around it, handle external connections as appropriate.
- WASI offers additional capabilities (direct hardware peripherals etc.) provided platforms are broken up. Offer **dynamic platforms** for WASI, WASM, Linux, or JavaScript exposing all capabilities a given interface/serving method could allow, **filtered** by deployment or by the capabilities requested for a specific world. In some environments, automatically download any variant that is a subset of the allowed permissions for the entire server/execution — directly or by compiling it; if compiled by a trusted source, it becomes available as a download going forward.

## 8. Programs, capabilities, and the platform surface

- Perhaps the program allows adding a variety of **capabilities and preloaded programs** plus the capabilities those programs require; downloads then only needed for things outside the core list that get built in. In theory, someone could use their **per-tenant database + the list of all IDs they're aware of** to generate a derivation containing all of them — compressed and shipped alongside the database as a **complete backup** — or generate the list of derivations and let another process merge them.
- This allows **sharing stores with trusted users**, easily extended to access levels, ACLs, restrictions. Artefact shapes: a roc library · a roc platform library · an IT/Rust library · a JavaScript library · a static binary from the JS or Rust code — customized to what's needed: the bare minimum, static or dynamic ID binary, wrapped with bash/roc/Python/JavaScript wrappers to pull something dynamically on boot (when missing from cache) — smallest initial transfer, paying additional round trips at launch, allowing dynamic boots and self-updating programs. Versions that do/don't include the roc compiler; a version without the compiler but with config setting a series of **trusted roc public keys** (trusted build caches / Nix stores providing builds or Nix code derivations and other files), broken down into sets, content-addressed per file or per file segment, up to per derivation.
- Generate a **platform for using Nix on a given machine**, exposed from admin apps that specifically ask for it, for admin/privileged/delegated users, with an interface to configure retention and: precompile · recompile · version update · metadata update · **usage statistics** (number of downloads, reported durations of use, reported success, no errors, reported errors with codes and messages, ID versions, geographic location, user ID of downloader/executor, participants, number of executions, length of executions, players and player counts, other outputs of executions) · performing backups to provided paths · generating the base Nix derivation and building it · using the base derivation to generate a Windows installer, a standalone website, and a WASI variant roc program · built with a dedicated ID binary by default, launchable as program or service with flags for dynamic instead of static · using the existing binary on PATH or from Nix packages · using/relying on an existing service or a provided socket, local URL, or remote URL+port · building and testing and recording results until we have a clear path for **bulletproof builds**.
- Usages: a library/dependency of both roc world scripts and roc user scripts · the ID binary · the ID service wrappers · installers.

## 9. If Nix loses

Early exploration may show these paths aren't as useful as they seem; use another format or database or Nix store for some aspects. Maybe a format to compress and handle **deltas and profiles** raw from a bucket-compatible API, appropriately resolving for the given store. Or other tooling better supporting deployments: with/without a resident ID binary · with/without an ID server · binaries connecting to remote servers · display-only interfaces · scripts that download all dependencies and binaries when missing.

## 10. What the store holds

What's stored in Nix is just the variety of different **wrappers and programs that can be merged together to create a preset**.
