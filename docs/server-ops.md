# Relay Server Operations

Production server operations for the relay infrastructure at `relay.lensacademy.org`.

## Server

- **IP:** 46.224.127.155
- **SSH:** `ssh -i ~/.ssh/hetzner_corec root@46.224.127.155`
- **Relay Server ID:** `cb696037-0f72-4e93-8717-4e433129d789`
- **Domain:** lensacademy.org (Cloudflare)

## Architecture

```
Internet → Cloudflare (HTTPS/443) ← cloudflared (outbound tunnel) → relay-server (HTTP/8080)
```

### Cloudflare Tunnel

- **Tunnel ID:** `78fd1b0b-09c6-460d-b9ab-672d3b337cac`
- Traffic routes through Cloudflare Tunnel (no inbound ports needed)
- cloudflared creates outbound-only connection to Cloudflare edge
- Configured in Cloudflare Zero Trust dashboard under Networks → Tunnels

### Docker Containers

1. **cloudflared** — Cloudflare Tunnel connector
   - Image: `cloudflare/cloudflared:latest`
   - Routes `relay.lensacademy.org` to `relay-server:8080`
   - Network: default compose network

2. **relay-server** — Main relay server (customized)
   - Image: `relay-server:custom` (built from modified source)
   - Port: 8080 (internal only, accessed via cloudflared)
   - Config: `/root/relay.toml`
   - Credentials: `/root/auth.env` (R2 access keys)
   - Storage: Cloudflare R2 bucket `lens-relay-storage`
   - Network: default compose network

3. **relay-git-sync** — Git synchronization service
   - Image: `docker.system3.md/relay-git-sync:latest` (with patched persistence.py)
   - Port: 8000 (internal only, receives webhooks from relay-server)
   - Config: `/root/relay-git-sync-data/git_connectors.toml`
   - Data: `/root/relay-git-sync-data/`
   - Network: default compose network

4. **lens-editor** — Web editor frontend + Discord bridge
   - Image: `lens-editor:custom` (built from lens-editor/)
   - Port: 3000 (internal, accessed via cloudflared)
   - Includes Discord API proxy (merged from discord-bridge sidecar)
   - Network: default compose network

## Cloudflare R2 Storage

- **Bucket:** `lens-relay-storage`
- **Account ID:** `a6d884b270b09f11bd79a048e42f88d2`
- **Endpoint:** `https://a6d884b270b09f11bd79a048e42f88d2.r2.cloudflarestorage.com`

**relay.toml storage configuration:**
```toml
[store]
type = "cloudflare"
account_id = "a6d884b270b09f11bd79a048e42f88d2"
bucket = "lens-relay-storage"
prefix = ""
```

**Credentials** in `/root/auth.env` (not in relay.toml for security):
```bash
AWS_ACCESS_KEY_ID=<access-key>
AWS_SECRET_ACCESS_KEY=<secret-key>
```

### R2 Storage Structure

Documents and attachments are stored separately with different UUIDs:

```
lens-relay-storage/
├── <relay-id>-<doc-uuid>/           # Documents
│   └── ...                          # CRDT data for each markdown file
└── files/
    └── <relay-id>-<attachment-uuid>/  # Attachments
        └── <filename>                 # Actual image/file data
```

**How attachments are linked:**
1. Markdown document has its own UUID (e.g., `076d2f81-...`)
2. Document content references attachment by filename: `![[my-image.webp]]`
3. `shared_folders.json` metadata maps filename → attachment UUID (e.g., `0c95356e-...`)
4. Attachment stored in R2 at `files/<relay-id>-<attachment-uuid>/`

**Inspecting storage:**
```bash
# List document folders
rclone lsd r2:lens-relay-storage/ | grep -v files

# List attachment folders
rclone lsd r2:lens-relay-storage/files/

# Check attachment metadata
cat /root/relay-git-sync-data/state/cb696037-0f72-4e93-8717-4e433129d789/shared_folders.json | python3 -c "
import sys,json
d = json.load(sys.stdin)
for folder_id, files in d.items():
    for path, meta in files.items():
        if meta.get('type') == 'image':
            print(f'{path}: {meta.get(\"id\")}')" | head -10
```

## Key Files

| File | Purpose |
|------|---------|
| `/root/relay.toml` | Relay server configuration |
| `/root/auth.env` | Cloudflare R2 credentials |
| `/root/relay-git-sync-data/git_connectors.toml` | Maps shared folders to GitHub repos |
| `/root/relay-git-sync-data/webhook_handler.py` | Patched webhook handler (mounted volume) |
| `/root/relay-git-sync-data/persistence.py` | Patched persistence.py for SSH config support |
| `/root/relay-git-sync-data/ssh/config` | SSH config with host aliases for multiple deploy keys |
| `/root/relay-git-sync-data/ssh/git_sync_key` | SSH key for lens-relay repo |
| `/root/relay-git-sync-data/ssh/educational_key` | SSH key for lens-educational-content repo |

## Git Sync

### Synced Folders

| Obsidian Folder | Shared Folder ID | GitHub Repo | Branch |
|-----------------|------------------|-------------|--------|
| Lens | `fbd5eb54-73cc-41b0-ac28-2b93d3b4244e` | [Lens-Academy/lens-relay](https://github.com/Lens-Academy/lens-relay) | main |
| Lens Edu | `ea4015da-24af-4d9d-ac49-8c902cb17121` | [Lens-Academy/lens-edu-relay](https://github.com/Lens-Academy/lens-edu-relay) | staging |

- **Sync interval:** Changes committed every ~10 seconds
- **Webhook endpoint:** https://relay.lensacademy.org/git-sync/webhooks

### Git Connector Config

`/root/relay-git-sync-data/git_connectors.toml`:
```toml
[[git_connector]]
shared_folder_id = "fbd5eb54-73cc-41b0-ac28-2b93d3b4244e"
relay_id = "cb696037-0f72-4e93-8717-4e433129d789"
url = "git@github.com:Lens-Academy/lens-relay.git"
branch = "main"
remote_name = "origin"
prefix = ""

[[git_connector]]
shared_folder_id = "ea4015da-24af-4d9d-ac49-8c902cb17121"
relay_id = "cb696037-0f72-4e93-8717-4e433129d789"
url = "git@github.com-educational:Lens-Academy/lens-edu-relay.git"
branch = "staging"
remote_name = "origin"
prefix = ""
```

### SSH Config for Multiple Repos

GitHub deploy keys can only be used for one repository. To sync multiple folders to different repos, we use SSH host aliases with separate keys.

`/root/relay-git-sync-data/ssh/config`:
```
# Default GitHub (lens-relay)
Host github.com
    HostName github.com
    User git
    IdentityFile /data/ssh/git_sync_key
    IdentitiesOnly yes

# Educational content repo
Host github.com-educational
    HostName github.com
    User git
    IdentityFile /data/ssh/educational_key
    IdentitiesOnly yes
```

**Adding a new repo:**
1. Generate a new SSH key: `ssh-keygen -t ed25519 -f /root/relay-git-sync-data/ssh/<new_key_name> -N ''`
2. Add the public key as a deploy key to the new GitHub repo (with write access)
3. Add a new Host block to `/root/relay-git-sync-data/ssh/config`
4. Add a new `[[git_connector]]` entry using the host alias (e.g., `git@github.com-newrepo:user/repo.git`)
5. Restart the container: `docker restart relay-git-sync`

**Patched persistence.py:** The stock relay-git-sync image hardcodes a single SSH key. We mount a patched `persistence.py` that uses `-F /data/ssh/config` instead, allowing SSH config-based key selection.

## Common Commands

```bash
# View logs
docker logs -f relay-server
docker logs -f cloudflared
docker logs -f relay-git-sync

# Restart services
docker restart relay-server
docker restart cloudflared
docker restart relay-git-sync

# Check running containers
docker ps -a

# View relay config
cat /root/relay.toml

# Test SSH connections from git-sync container
docker exec relay-git-sync ssh -F /data/ssh/config -T git@github.com
docker exec relay-git-sync ssh -F /data/ssh/config -T git@github.com-educational

# Manual sync of a specific folder
docker exec relay-git-sync uv run python cli.py sync \
  --relay-id cb696037-0f72-4e93-8717-4e433129d789 \
  --folder-id <folder-uuid>
```

## Content Validation

Educational content is validated using a TypeScript parser/validator published as an npm package.

- **Package:** [lens-content-processor](https://www.npmjs.com/package/lens-content-processor) (v0.1.0)
- **Source:** Part of the lens-platform repo (`content_processor/`)

```bash
# Validate a content directory
npx lens-content-processor ./path/to/content --output result.json
```

### GitHub Actions Workflow

The `lens-edu-relay` repository has automated validation via `.github/workflows/validate.yml`:
- Pull requests to `main`, manual dispatch, scheduled every 5 minutes (validates `staging` branch)
- Skips if commit is < 5 minutes old or already validated

**Modifying the workflow** — never push directly to the educational content repo. Edit via the relay server:
```bash
ssh -i ~/.ssh/hetzner_corec root@46.224.127.155

vim /root/relay-git-sync-data/repos/cb696037-0f72-4e93-8717-4e433129d789/ea4015da-24af-4d9d-ac49-8c902cb17121/.github/workflows/validate.yml

docker exec relay-git-sync bash -c 'cd /data/repos/cb696037-0f72-4e93-8717-4e433129d789/ea4015da-24af-4d9d-ac49-8c902cb17121 && git add -A && git commit -m "Update workflow" && GIT_SSH_COMMAND="ssh -F /data/ssh/config" git push origin staging'
```

## Known Issues

### WebSocket File Descriptor Leak

The relay-server leaks socket file descriptors when WebSocket connections close. Sockets accumulate in CLOSE-WAIT state (~70/hour).

**Root cause:** Bug in relay-server (y-sweet) code — doesn't call close() on its end of TCP connections when the remote side closes.

**Workaround:** Increased FD limit to 65536 (from default 1024). Extends time-to-crash from ~14 hours to ~39 days.

### FD Monitoring

A cron job monitors FD usage every 5 minutes.

- **Log:** `/var/log/relay-fd-monitor.log`
- **Scripts:** `/usr/local/bin/relay-fd-monitor.sh`, `/usr/local/bin/relay-fd-report.sh`

```bash
# Quick report with trends
/usr/local/bin/relay-fd-report.sh

# Watch in real-time
tail -f /var/log/relay-fd-monitor.log
```

## Deploying / Updating

All services are managed via `docker-compose.prod.yaml` at the repo root:

```bash
# Deploy/update all services
docker compose -f docker-compose.prod.yaml build
docker compose -f docker-compose.prod.yaml up -d

# Rebuild a single service
docker compose -f docker-compose.prod.yaml build relay-server
docker compose -f docker-compose.prod.yaml up -d relay-server
```

See `.env.example` for required environment variables.

## Troubleshooting

- **FD exhaustion:** Run `/usr/local/bin/relay-fd-report.sh`. If high, `docker restart relay-server`.
- **Container not starting:** Check `docker logs <container>`.
- **Connectivity:** `curl -v https://relay.lensacademy.org`. Check tunnel status in Cloudflare Zero Trust dashboard.
- **Git sync not working:** Check `docker logs --tail 50 relay-git-sync`. Test SSH keys from container. Verify deploy keys have write access on GitHub.
