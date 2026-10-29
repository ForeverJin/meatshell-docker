# meatshell-docker

A **headless** cut of [meatshell](https://github.com/ForeverJin/meatshell)
(upstream v0.8.0, commit `5c39ca0`) for servers, NAS boxes and containers.

Kept:

- `meatshell cli ...` — session import, session listing, remote command
  execution, SFTP list/read/upload/download
- `meatshell mcp serve` — local stdio MCP plus an OAuth 2.1 protected
  Streamable HTTP MCP endpoint (`--http-config`; see
  [`docs/REMOTE_MCP.md`](docs/REMOTE_MCP.md))
- The SSH/SFTP runtime: jump hosts, tunnels, proxies, PPK keys, FinalShell
  import, and the existing profile/credential format

Removed:

- The whole Slint UI (`src/app`, `src/ui`, `ui/*.slint`, `lang/`, fonts, icons)
- Desktop-only modules: layout, wallpapers, WebDAV browser, RDP, resource
  monitor, terminal renderer, local PTY/serial tabs, tray icon, auto-updater
- Their dependencies: `slint`, `i-slint-*`, `tray-icon`, `rfd`, `arboard`,
  `fontdb`, `twemoji-assets`, `image`, `serialport`, `portable-pty`, `ureq`, ...
- Desktop packaging (deb/Flatpak/AUR/WiX) and the three-platform release jobs

The shared runtime still carries some GUI-era API. Those modules are silenced
with `#[allow(dead_code)]` (see the comment at the top of `src/main.rs`) so
upstream merges stay cheap; they can be pruned once upstream sync ends.

## Build

No GTK/X11/Wayland packages are needed — only a C toolchain (jemalloc builds C):

```bash
sudo apt install -y build-essential pkg-config
cargo build --release          # target/release/meatshell
cargo test --bin meatshell     # 215 unit tests
```

Rust >= 1.88.

## Use

```bash
DATA=/var/lib/meatshell
./meatshell --data-dir "$DATA" cli help
./meatshell --data-dir "$DATA" cli sessions --json
./meatshell --data-dir "$DATA" cli exec <session-id> -- uptime
./meatshell --data-dir "$DATA" mcp serve
./meatshell --data-dir "$DATA" mcp serve --http-config /etc/meatshell/http.json
```

`MEATSHELL_DATA_DIR` can replace `--data-dir`; `MEATSHELL_LANG=en` switches the
built-in messages to English (Chinese by default).

The MCP service cannot show a host-key prompt: unknown or changed SSH host keys
fail closed, so seed the verified public keys into the profile `known_hosts`
(MeatShell's own `host:port key-type base64` format, not OpenSSH's) before the
first connection. Deployment details, OAuth requirements and the reverse-proxy
setup are in [`docs/REMOTE_MCP.md`](docs/REMOTE_MCP.md).
