# Lens Relay Server (Monorepo)

Fork of [No-Instructions/relay-server](https://github.com/No-Instructions/relay-server) with custom HMAC auth fixes, link indexer, and the lens-editor web client.

## Architecture

```
                    ┌─────────────────────┐
                    │   Cloudflare R2     │
                    │ (lens-relay-storage) │
                    └────────┬────────────┘
                             │
Internet ── Cloudflare ── cloudflared ── relay-server (Rust, port 8080)
               Tunnel                        │
                                             │ webhooks
                                             ▼
                                       relay-git-sync
                                        │         │
                                        ▼         ▼
                                   lens-relay  lens-edu-relay
                                   (GitHub)    (GitHub)

Clients:
  - Obsidian + Relay.md plugin (real-time collaborative editing)
  - lens-editor (web-based editor, React + CodeMirror)
```

### Monorepo Layout

```
crates/               # Relay server (Rust, upstream y-sweet fork)
  relay/              #   Main server binary
  y-sweet-core/       #   Core CRDT/auth logic
  y-sign/             #   Token signing CLI
  Dockerfile          #   Production Docker build
lens-editor/          # Web editor (React + CodeMirror + yjs)
docs/                 # Operational documentation
```

## Components

| Component | Location | Description |
|-----------|----------|-------------|
| **relay-server** | `crates/` | Rust-based CRDT sync server (y-sweet). Custom HMAC auth fixes for service accounts. |
| **lens-editor** | `lens-editor/` | Web-based editor for relay documents. React + CodeMirror + yjs. Connects to relay-server via WebSocket. |
| **relay-git-sync** | External: `No-Instructions/relay-git-sync` | Syncs relay shared folders to GitHub repos via webhooks. Runs as Docker container on production server. |
| **Relay.md plugin** | External: `No-Instructions/Relay` | Obsidian plugin for real-time collaboration via relay-server. |

## Infrastructure

- **Relay server URL:** https://relay.lensacademy.org
- **Production server:** Hetzner VPS (46.224.127.155), Docker containers
- **Storage:** Cloudflare R2 bucket `lens-relay-storage`
- **Tunnel:** Cloudflare Tunnel (no inbound ports needed)
- **Relay ID:** `cb696037-0f72-4e93-8717-4e433129d789`

## Running relay-server

### With Docker (production-like)

```bash
docker build -t relay-server:custom -f crates/Dockerfile crates/
docker run -d \
  --name relay-server \
  --restart unless-stopped \
  --network relay-network \
  --ulimit nofile=65536:524288 \
  -v /root/relay.toml:/app/relay.toml:ro \
  --env-file /root/auth.env \
  relay-server:custom
```

### With Cargo (local dev)

```bash
cargo run --manifest-path=crates/Cargo.toml -- serve relay.toml
```

Requires a `relay.toml` config file. See `crates/relay.toml.example` for format.

## Running lens-editor

```bash
cd lens-editor && npm install && npm run dev
```

The editor connects to the relay server at the URL configured in its settings. During development, point it at `https://relay.lensacademy.org` or a local relay-server instance.

See `lens-editor/CLAUDE.md` for Y.Doc structure documentation and editor-specific development guidance.

## Upstream Sync

The `upstream` remote tracks `No-Instructions/relay-server`. Our additions (`lens-editor/`, `docs/`) don't exist upstream, so merges are clean.

```bash
# Fetch upstream changes
jj git fetch --remote upstream

# Rebase our work on top
jj rebase -s <our-first-custom-change> -d upstream/main
```

## Custom Relay Server Changes

Our fork adds two categories of changes on top of upstream:

**HMAC auth fixes** (enables service accounts to coexist with Relay.md client auth):
- `gen_doc_token_auto()` / `gen_file_token_auto()` — auto-detect key type for token generation
- File token generation for server/prefix tokens in download URLs

See [docs/relay-auth-customizations.md](docs/relay-auth-customizations.md) for full details.

**Link indexer:**
- Wikilink extraction from Y.Doc content
- Backlink tracking
- Folder-content mapping for multi-folder support

## Git Sync

Two shared folders are synced to GitHub:

| Obsidian Folder | GitHub Repo | Branch |
|-----------------|-------------|--------|
| Lens | [Lens-Academy/lens-relay](https://github.com/Lens-Academy/lens-relay) | main |
| Lens Edu | [Lens-Academy/lens-edu-relay](https://github.com/Lens-Academy/lens-edu-relay) | staging |

See [docs/server-ops.md](docs/server-ops.md) for git connector config, SSH key setup, and operational details.

## Known Issues

- **WebSocket FD leak** in relay-server (sockets accumulate in CLOSE-WAIT). Workaround: `--ulimit nofile=65536:524288` extends time-to-restart to ~39 days.

## Version Control

This repo uses non-colocated jj. See `~/.claude/jj.md` for workflow reference.

Personal/local overrides go in `CLAUDE.local.md` (gitignored). Symlinked from parent directory in workspace setups.
