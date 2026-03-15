# id

An iroh-based peer-to-peer file sharing CLI tool.

## Overview

`id` is a command-line tool for storing and sharing files using content-addressed storage and peer-to-peer networking. Built on [iroh](https://iroh.computer/), it provides:

- **Content-addressed storage**: Files are identified by their cryptographic hash
- **Named tags**: Human-friendly names for your files
- **Peer-to-peer transfers**: Share files directly with other nodes
- **Interactive REPL**: Shell-like interface with command substitution
- **Background server**: Long-running process for accepting connections
- **Fuzzy search**: Find files by partial name or hash matches
- **Content preview**: Peek at file contents with head/tail display

## Installation

```bash
# Clone and build
git clone <repository>
cd pkgs/id
cargo build --release

# Or install directly
cargo install --path .
```

## Quick Start

```bash
# Store a file
id put myfile.txt

# List stored files
id list

# Retrieve a file
id get myfile.txt

# Search for files
id search config

# Show file content by pattern
id show config

# Preview a file
id peek readme

# Start interactive REPL
id repl

# Start a server (for remote access)
id serve
```

## Commands

### Storage Commands

| Command | Description |
|---------|-------------|
| `put <FILE> [NAME]` | Store a file with optional custom name |
| `put-hash <FILE>` | Store file by hash only (no name) |
| `get <NAME> [OUTPUT]` | Retrieve file by name |
| `get-hash <HASH> <OUTPUT>` | Retrieve file by hash |
| `cat <NAME>` | Output file to stdout |

### Search Commands

| Command | Description |
|---------|-------------|
| `find <QUERY>` | Find and output matching files |
| `search <QUERY>` | List matches without content |
| `show <QUERY>` | Find and output content (alias: `view`) |
| `peek <QUERY>` | Preview with head/tail display |
| `list` | List all stored files |

### System Commands

| Command | Description |
|---------|-------------|
| `serve` | Start background server |
| `repl` | Start interactive REPL |
| `id` | Print this node's public ID |

## Search Filtering

All search commands (`find`, `search`, `show`, `peek`) support filtering flags:

```bash
# Get first 3 matches
id search --first 3 config

# Get last 5 matches
id search --last 5 config

# Count matches
id search --count config

# Exclude patterns (repeatable)
id search --exclude .bak --exclude .tmp config

# Combine filters
id search --first 10 --exclude .bak config
```

### Filter Flags

| Flag | Description |
|------|-------------|
| `--first [N]` | Return first N matches (default 1 if N omitted) |
| `--last [N]` | Return last N matches (default 1 if N omitted) |
| `--count` | Print count instead of matches |
| `--exclude PATTERN` | Exclude matches containing pattern |

## Show/View Command

Output file content found by pattern search:

```bash
# Show first match
id show config

# Show all matches
id show --all config

# Write to file
id show -o output.txt config

# With filtering
id show --first 3 --exclude .bak config
```

## Peek Command

Preview files with configurable head/tail display:

```bash
# Default: 5 head + 5 tail lines
id peek readme

# Custom line count
id peek -n 10 readme

# Head only
id peek --head-only -n 20 readme

# Tail only
id peek --tail-only -n 20 readme

# Preview by characters
id peek --chars -n 100 readme

# Preview by words
id peek --words -n 50 readme

# Quiet mode (no header)
id peek -q readme

# Output to file
id peek -o preview.txt readme
```

## Remote Operations

Commands support remote operations by specifying a 64-character hex node ID:

```bash
# Get from remote node
id get <NODE_ID> config.json

# Put to remote node
id put <NODE_ID> myfile.txt

# List files on remote node
id list <NODE_ID>

# Search on remote node
id search --node <NODE_ID> config
```

## REPL Features

The interactive REPL supports shell-like features:

```bash
# Start REPL
id repl

# Command substitution
> put $(date +%Y-%m-%d) today.txt

# Backtick substitution
> put `cat file.txt` content.txt

# Pipe operator
> echo "hello" |> put - greeting.txt

# Here-string
> put - note.txt <<< 'Quick note'

# Heredoc
> put - story.txt <<EOF
Once upon a time...
The end.
EOF

# Shell escape
> !ls -la

# Remote targeting
> list @<NODE_ID>
> get @<NODE_ID> config.json

# Search commands in REPL
> find config
> search --first 5 config
> show readme
> peek --lines 10 config
```

### REPL Search Flags

In the REPL, search commands support the same filtering flags:

```bash
> search --first 5 config
> search --last 3 config
> search --count config
> search --exclude .bak config
> find --first 2 --exclude .tmp readme
> show --all config
> peek --head-only -n 10 readme
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         CLI Layer                           │
│  (main.rs → cli.rs → Command dispatch)                     │
└─────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
   ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
   │   commands/ │     │    repl/    │     │   serve     │
   │  (put/get/  │     │ (input.rs,  │     │  (server,   │
   │  find/etc.) │     │  runner.rs) │     │  protocol)  │
   └─────────────┘     └─────────────┘     └─────────────┘
          │                   │                   │
          └───────────────────┼───────────────────┘
                              ▼
   ┌─────────────────────────────────────────────────────────┐
   │                      Core Layer                          │
   │  lib.rs: Constants, utilities, exports                  │
   │  store.rs: Blob storage (FsStore/MemStore)              │
   │  protocol.rs: Network protocol (MetaRequest/Response)   │
   │  helpers.rs: Parsing and formatting                     │
   └─────────────────────────────────────────────────────────┘
                              │
                              ▼
   ┌─────────────────────────────────────────────────────────┐
   │                   iroh / iroh-blobs                      │
   │  Content-addressed storage + QUIC networking            │
   └─────────────────────────────────────────────────────────┘
```

## Data Storage

- **Store path**: `.iroh-store/` (SQLite database)
- **Key files**:
  - `.iroh-key` - Server identity keypair
  - `.iroh-client-key` - Client identity keypair
  - `.iroh-serve.lock` - Server lock file with connection info

## Protocol

Communication uses two ALPN protocols:

- **META_ALPN** (`id/meta/1`): Metadata operations (list, put, get, find, etc.)
- **BLOBS_ALPN** (`iroh/blobs/...`): Binary blob transfers

## API Documentation

Full API documentation is available via rustdoc:

```bash
cargo doc --open
```

## License

MIT

---

## Original Brainstorming

```
# TypeCharacteristics
# TypeDetails
Type
CreateType

# TypeLinkCharacteristics
# TypeLinkDetails
# TypeLinkType
TypeLink
CreateTypeLink

# NamespaceCharacteristics
# NamespaceDetails
# NamespaceType
Namespace
CreateNamespace

# url/uri/urn whatever
# url = namespace://(path(?(key=(value(,).)+)+).).
# named/custom ids
# custom-namespaces for specific content types or user-provided/managed
# hash-based ids different formats including custom treed hash of structured data kdl/json/xml/automerge (each part breaks out into it's own entity)
# resource-based/registry? specific format rules/etc.
# random-based id cuid2 / uuidv1/etc.
# time-based id cuid1 / uuidv6/7/8? / etc. with/without partitioning/subsecond
# versioned/semver/git/scm/nix/flakes/docker/npm/pip/nuget/go/etc.

# IdCharacteristics
# IdDetails
# IdType
Id
CreateId

# Hash

#FormatCharacteristics
#FormatDetails
#FormatType
Format
CreateFormat

#ContentCharacteristics
#ContentDetails
#ContentType
Content
CreateContent

# RawContent 
# content has a url to data,
# maybe a hash / format
# for data platform has interned it could be in rawcontent
# this could be a cached replacement or the primary source
# which may be a sql table, redis cache,
#   large files on local disk
#   s3 path
#   websocket call/response or logs
#   hashmap / in-ram cache
#   

# Entity

#LinkCharacteristics
#LinkDetails
#LinkType?
Link
CreateLink

#LicenseCharacteristics?
#LicenseDetails?
#LicenseType?
License
CreateLicense
#LicenseCharacteristics?

#RoomCharacteristics?
#RoomDetails?
#RoomType?
Room : { name : string, location : list(string)|string }
GetRoom(string id) : Room
GetDefaultRoom() : GetRoom("")
GetLocation(list(string)|string) : Location
CreateRoom


# Something to control privileges
# globally at the platform level
# per-entity/content
# for now:
#   read: all is public, maybe 2 levels admin/read, secrets stored in admin and admin can see them
#   write: only admins or the owner of named id can 'edit'. hashed things can't be edited. a new object can be made. delete and replaces links can be added.


# Something to control tokens/utilization
# tigerbeetle account controlled by owner of given id, 'fiat://id_url', infinite credits of fiat:id_url for owner, ability to give credits to other ids, no ability to revoke credits or it's separate privilege and impossible for some accounts.. once an account is made for an id that user can always trade/receive in that token. empty accounts may be deleted by user-owners..
# deletion is a marker on the account, nothing happens to any data.



# querying
# ways to return more than one result, query a specific attribute or id that may have more than one definition
# query down a tree following links of type X (callback function to allow any kind of traversal)



# retention policies
# indexed vs kept
# ttl based on access/edit
# ram/hashmap -> ring buffer -> local file optane/nvme -> redis -> websocket -> optane db (sqlite/duck/postgres) -> nvme db (sqlite/duck/postgres) -> optane file -> nvme file -> nvme seaweed -> hdd seaweed
# delete tag -> prune/compress -> pre-emptive index/cache
# lru cache of pull/used // on-startup file // Ids_Seen // streamed/published by controller
```
