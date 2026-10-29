# meatshell-docker

A **headless** cut of [meatshell](https://github.com/ForeverJin/meatshell)
(upstream v0.8.0, commit `5c39ca0`): SSH/SFTP automation, an authenticated MCP
server, and a browser UI for managing connections — for servers, NAS boxes and
containers.

| Capability | Entry point |
| --- | --- |
| Connection manager web UI | `http://<host>:8765/`, log in with an MCP token |
| Remote MCP (OAuth 2.1 + Streamable HTTP) | `POST /mcp` |
| Local stdio MCP | `meatshell mcp serve` |
| Scriptable CLI | `meatshell cli ...` |
| Container image | `docker compose up -d` (109 MB, non-root, read-only rootfs) |

Kept from upstream: the SSH/SFTP runtime, jump-host chains, SOCKS/HTTP proxies,
PPK keys, FinalShell import, ZMODEM receive, encrypted session storage. Profiles
and credential formats are unchanged, so an existing desktop profile works as-is.

Removed: the whole Slint UI (`src/app`, `src/ui`, `ui/*.slint`, `lang/`, fonts,
icons) plus layout, wallpapers, WebDAV browser, RDP, resource monitor, terminal
renderer, serial/local PTY tabs, tray icon and auto-updater — and their
dependencies (`slint`, `i-slint-*`, `tray-icon`, `rfd`, `arboard`, `fontdb`,
`twemoji-assets`, `image`, `serialport`, `portable-pty`, ...), desktop packaging
(deb/Flatpak/AUR/WiX) and the three-platform release workflows.

The shared runtime still carries some GUI-era API; those modules are silenced
with `#[allow(dead_code)]` (see the comment at the top of `src/main.rs`) so
upstream merges stay cheap.

## Quick start (Docker)

```bash
git clone https://github.com/ForeverJin/meatshell-docker.git
cd meatshell-docker

mkdir -p data && chmod 700 data
python3 docker/mint-token.py init \
  --config-dir config --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ \
  --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang

cp .env.example .env      # ports / UID / registry mirror
docker compose build && docker compose up -d
docker compose ps         # wait for healthy
```

Open `http://192.168.8.114:8765/` and paste a token:

```bash
python3 docker/mint-token.py mint --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ \
  --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang --ttl 0        # 0 = never expires
```

MCP clients use the same token as `Authorization: Bearer <token>` against
`http://<host>:8765/mcp`. Full configuration, reverse proxy, `known_hosts`
seeding, backup and troubleshooting: [`docker/README.md`](docker/README.md).

> The default deployment is plain HTTP on a private LAN. For anything reachable
> from outside, run the compose `proxy` profile (Caddy + TLS) and change
> `resource` to `https://your.domain/mcp`.

## Web UI

Served on the MCP listener with the same authentication.

- **Login** — paste a bearer token; the server verifies it with the same OAuth
  configuration and sets an HttpOnly, SameSite=Strict session cookie (the token
  never reaches browser storage).
- **Manage** — list/search/group sessions; create, edit, delete; password,
  keyboard-interactive or private key (container path or pasted inline key).
- **Test** — a real SSH handshake plus authentication, reporting success, auth
  failure, missing credentials or an MFA requirement.
- **Host keys** — unknown or changed keys show their `SHA256:` fingerprint; after
  checking it against the server console, "trust and retry" stores exactly the
  key that was displayed in `data/known_hosts`.
- **Write-only credentials** — the API never returns passwords or keys, only
  "has password / has key" markers.
- Disable with `"web_ui": false` in `config/http.json`.

Anyone who can log in has full control of the profile (add/remove connections,
make the service use stored credentials), so treat the token as an admin
password. The page sets CSP and `X-Frame-Options: DENY`; writes require the
`X-Ms-Web` header and a matching Origin.

## MCP

The HTTP transport exposes eight tools: `import_sessions`, `list_sessions`,
`get_session`, `run_command`, `list_remote_files`, `read_remote_text_file`,
`upload_file`, `download_file`.

```bash
./meatshell --data-dir /var/lib/meatshell mcp serve \
  --http-config /etc/meatshell/http.json
```

The service only verifies tokens; it never issues them. Issuer, resource, JWKS
and subject requirements are in [`docs/REMOTE_MCP.md`](docs/REMOTE_MCP.md). There
is no interactive host-key prompt, so trust must be seeded first — the web UI's
"trust and retry" is the easiest way.

## CLI

```bash
DATA=/var/lib/meatshell
./meatshell --data-dir "$DATA" cli help
./meatshell --data-dir "$DATA" cli import export.json --dry-run
./meatshell --data-dir "$DATA" cli sessions --json
./meatshell --data-dir "$DATA" cli exec <session-id> -- uptime
./meatshell --data-dir "$DATA" cli files <session-id> /etc
./meatshell --data-dir "$DATA" cli upload <session-id> local.txt /tmp
./meatshell --data-dir "$DATA" cli download <session-id> /tmp/a.log ./downloads
```

`MEATSHELL_DATA_DIR` can replace `--data-dir`; `--config-info` prints the profile
path, version and session count; `MEATSHELL_LANG=en` switches built-in messages
to English (Chinese by default).

## Build from source

No GTK/X11/Wayland packages are needed — only a C toolchain (jemalloc builds C):

```bash
sudo apt install -y build-essential pkg-config
cargo build --release          # target/release/meatshell
cargo test --bin meatshell     # 216 unit tests
cargo check --all-targets      # no warnings
```

Rust >= 1.88. The image build accepts `CARGO_REGISTRY_MIRROR` and
`BUILDER_BASE` / `RUNTIME_BASE` for slow or blocked networks.

## Data and credentials

Everything lives in the profile directory (`/data` in the container, `./data` on
the host):

| File | Contents |
| --- | --- |
| `sessions.json` | sessions; passwords and inline keys encrypted with ChaCha20-Poly1305 (`enc:v1:` prefix) |
| `secret.key` | profile key (0600, generated on first run) |
| `known_hosts` | host trust in MeatShell's own `host:port type base64` format |
| `log/` | runtime logs |

Back up `sessions.json` with `secret.key` to restore saved credentials; losing
`secret.key` makes old passwords unrecoverable. Passwords inside export files use
a fixed key compiled into the binary (obfuscation only).

## Layout

```
src/
  main.rs           entry point: cli / mcp / --version
  cli/              CLI commands
  mcp/              stdio + Streamable HTTP MCP, OAuth/JWKS verification
    impls/webui.rs      connection manager API and auth
    impls/webui.html    embedded single-page frontend (no build step)
  automation/       shared automation used by CLI, MCP and the web UI
  ssh/              SSH, jump hosts, proxies, PPK, known_hosts, connection probe
  sftp/             SFTP transfers and elevation
  config/           profile, sessions.json, import/export, encrypted credentials
  terminal/         encoding adapter and ZMODEM receive only
  tunnel/           port forwarding
  logging/          file logging
docker/             container deployment, token helper, Caddy template
docs/REMOTE_MCP.md  remote MCP deployment notes
tests/              Python end-to-end regressions (HTTP/OAuth, import, jump chains)
```

## Limitations

- Web UI connection tests support SSH only; MFA/OTP servers cannot complete a
  login there.
- `private_key_path` resolves inside the container, so that directory must be
  mounted; inline keys avoid this.
- Single-owner model: every authorized subject shares one profile.
- Desktop-only features (RDP, serial, local shells, resource panel, multi-tab
  window) are not part of this repository.
