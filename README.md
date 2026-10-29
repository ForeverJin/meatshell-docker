# meatshell-docker

从 [meatshell](https://github.com/ForeverJin/meatshell)（上游 v0.8.0，提交
`5c39ca0`）裁剪出来的**无界面版本**，用于在服务器 / NAS / 容器里跑
SSH/SFTP 自动化与 MCP 服务。

保留的东西：

- `meatshell cli ...`：会话导入、会话列表、远程执行命令、SFTP 列目录/读/上传/下载
- `meatshell mcp serve`：本地 stdio MCP，以及带 OAuth 2.1 校验的远程
  Streamable HTTP MCP（`--http-config`，见 [`docs/REMOTE_MCP.md`](docs/REMOTE_MCP.md)）
- SSH / SFTP / 跳板机 / 隧道 / 代理 / PPK 私钥 / FinalShell 导入等运行时
- 配置与凭据存储格式保持不变，可直接复用桌面版的 profile

去掉的东西：

- 全部 Slint 界面（`src/app`、`src/ui`、`ui/*.slint`、`lang/`、字体与图标资源）
- 桌面相关模块：布局、壁纸、WebDAV 浏览、RDP、资源监控面板、终端渲染器、
  串口/本地 PTY 标签、系统托盘、自动更新器
- 对应依赖：`slint`、`i-slint-*`、`tray-icon`、`rfd`、`arboard`、`fontdb`、
  `twemoji-assets`、`image`、`serialport`、`portable-pty`、`ureq` 等
- 桌面打包配置（deb/Flatpak/AUR/WiX）与三平台发布工作流

GUI 时代的共享运行时（`ssh`、`sftp`、`config`、`tunnel`）里仍留有只被桌面版
调用的 API。这些模块暂时用 `#[allow(dead_code)]` 抑制告警（见 `src/main.rs`
顶部注释），便于后续与上游继续同步；等确认不再合并上游后可以再清理一轮。

## 构建

不需要 GTK / X11 / Wayland 等桌面开发包，只需要 C 工具链（jemalloc 要编译 C）：

```bash
sudo apt install -y build-essential pkg-config
cargo build --release          # 产物 target/release/meatshell
cargo test --bin meatshell     # 215 个单元测试
```

要求 Rust ≥ 1.88。

## 使用

```bash
DATA=/var/lib/meatshell

./meatshell --version
./meatshell --data-dir "$DATA" cli help
./meatshell --data-dir "$DATA" cli import export.json --dry-run
./meatshell --data-dir "$DATA" cli sessions --json
./meatshell --data-dir "$DATA" cli exec <session-id> -- uptime
./meatshell --data-dir "$DATA" cli files <session-id> /etc
```

`--data-dir` 也可以用环境变量 `MEATSHELL_DATA_DIR` 指定；`--config-info` 打印
当前 profile 路径、版本和会话数。界面语言通过 `MEATSHELL_LANG=en` 切到英文
（默认中文）。

本地 stdio MCP：

```bash
./meatshell --data-dir "$DATA" mcp serve
```

远程 HTTP MCP（必须显式指定 profile，且需要外部 OAuth 提供方与 HTTPS 反代）：

```bash
./meatshell --data-dir "$DATA" mcp serve --http-config /etc/meatshell/http.json
```

配置字段、JWKS 要求、`known_hosts` 预置格式和反向代理注意事项都在
[`docs/REMOTE_MCP.md`](docs/REMOTE_MCP.md)。注意 MCP 服务无法弹出主机密钥
确认框：未知或变更的 SSH 主机密钥一律拒绝，首次连接前必须把已验证的主机
公钥写入 profile 的 `known_hosts`（MeatShell 自有格式，不是 OpenSSH 格式）。

## 目录结构

```
src/
  main.rs         入口：cli / mcp / --version
  cli/            CLI 命令实现
  mcp/            stdio + Streamable HTTP MCP，OAuth/JWKS 校验
  automation/     GUI 与 CLI/MCP 共用的自动化能力（会话、执行、SFTP）
  ssh/            SSH/跳板/代理/PPK/known_hosts
  sftp/           SFTP 传输与提权
  config/         profile、sessions.json、导入导出、加密凭据
  terminal/       仅保留编码适配与 ZMODEM 接收
  tunnel/         端口转发
  logging/        文件日志
docs/REMOTE_MCP.md  远程 MCP 部署说明
tests/              Python 端到端回归（HTTP/OAuth、配置导入、跳板链）
```

## 容器化

```bash
mkdir -p data && chmod 700 data
python3 docker/mint-token.py init --config-dir config --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang
docker compose build && docker compose up -d
```

服务在同一端口还提供连接管理的 Web 页面（`http://<host>:8765/`，用 MCP token
登录），可以增删改连接、测试连通性并确认主机密钥指纹。

镜像基于 `debian:bookworm-slim`，只包含 CLI/MCP 二进制（无 GUI 依赖），以非
root、只读根文件系统运行。配置、反向代理、`known_hosts` 预置、令牌签发和
运维说明都在 [`docker/README.md`](docker/README.md)。
