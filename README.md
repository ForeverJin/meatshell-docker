# meatshell-docker

从 [meatshell](https://github.com/ForeverJin/meatshell)（上游 v0.8.0，提交
`5c39ca0`）裁剪出来的**无界面版本**：在服务器 / NAS / 容器里跑 SSH/SFTP
自动化、MCP 服务，并带一个浏览器里用的连接管理界面。

| 能力 | 入口 |
| --- | --- |
| 连接管理 Web UI | `http://<host>:8765/`，用 MCP token 登录 |
| 远程 MCP（OAuth 2.1 + Streamable HTTP） | `POST /mcp` |
| 本地 stdio MCP | `meatshell mcp serve` |
| 脚本化 CLI | `meatshell cli ...` |
| Docker 镜像 | `docker compose up -d`（109 MB，非 root、只读根文件系统） |

保留的运行时：SSH、SFTP、跳板机链、SOCKS/HTTP 代理、PPK 私钥、FinalShell
导入、ZMODEM 接收、session 加密存储。配置与凭据格式与桌面版一致，可以直接
复用已有 profile。

去掉的东西：全部 Slint 界面（`src/app`、`src/ui`、`ui/*.slint`、`lang/`、字体
图标资源），以及布局、壁纸、WebDAV 浏览、RDP、资源监控面板、终端渲染器、
串口/本地 PTY、系统托盘、自动更新器，和 `slint`/`i-slint-*`/`tray-icon`/
`rfd`/`arboard`/`fontdb`/`twemoji-assets`/`image`/`serialport`/`portable-pty`
等依赖，以及 deb/Flatpak/AUR/WiX 打包与三平台发布工作流。

GUI 时代的共享运行时（`ssh`、`sftp`、`config`、`tunnel`）里仍留有只被桌面版
调用的 API。这些模块暂时用 `#[allow(dead_code)]` 抑制告警（见 `src/main.rs`
顶部注释），便于后续与上游同步；确认不再合并上游后可以再清理一轮。

## 快速开始（Docker）

```bash
git clone https://github.com/ForeverJin/meatshell-docker.git
cd meatshell-docker

mkdir -p data && chmod 700 data
python3 docker/mint-token.py init \
  --config-dir config --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ \
  --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang

cp .env.example .env      # 按需修改端口 / UID / 镜像源
docker compose build && docker compose up -d
docker compose ps         # 等到 status 为 healthy
```

然后打开 `http://192.168.8.114:8765/`，粘贴 token 登录：

```bash
python3 docker/mint-token.py mint --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ \
  --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang --ttl 0        # 0 = 永不过期
```

MCP 客户端把同一个 token 放进 `Authorization: Bearer <token>`，地址是
`http://<host>:8765/mcp`。完整的字段说明、反向代理、`known_hosts` 预置、
备份与排错见 [`docker/README.md`](docker/README.md)。

> 默认只用于局域网明文 HTTP。对外部署请用 compose 的 `proxy` profile 起
> Caddy 终止 TLS，并把 `resource` 改成 `https://你的域名/mcp`。

## Web 管理界面

与 MCP 同端口、同认证体系，浏览器里不需要安装任何东西。

- **登录**：粘贴 Bearer token，服务用 `http.json` 的 OAuth 配置校验，成功后
  下发 HttpOnly + SameSite=Strict 的会话 cookie（token 不落浏览器存储）。
- **连接管理**：列表 / 搜索 / 分组；新建、编辑、删除；密码、键盘交互或私钥
  （容器内路径或直接粘贴私钥内容）。
- **测试连接**：真实 SSH 握手 + 认证，给出成功、认证失败、缺凭据、需要 MFA
  等明确结果。
- **主机密钥**：未知或变更的密钥会显示 `SHA256:...` 指纹；核对服务器控制台后
  点“信任并重试”，只把刚刚展示的那把公钥写入 `data/known_hosts`。
- **凭据只写不回读**：接口不返回密码/私钥，只返回“已存密码/已存密钥”标记。
- 不想要页面时在 `config/http.json` 里设 `"web_ui": false` 后重启。

安全边界：能登录页面等价于拿到该 profile 的完整操作权（可增删连接、可让服务
用已存凭据连 SSH），token 要当管理员密码保管；页面已启用 CSP、
`X-Frame-Options: DENY`，写操作要求 `X-Ms-Web` 头并校验 Origin。

## MCP

远程模式（Streamable HTTP，OAuth 2.1 资源服务器）提供 8 个工具：
`import_sessions`、`list_sessions`、`get_session`、`run_command`、
`list_remote_files`、`read_remote_text_file`、`upload_file`、`download_file`。

```bash
./meatshell --data-dir /var/lib/meatshell mcp serve \
  --http-config /etc/meatshell/http.json
```

服务只校验 token，不签发 token；`issuer` / `resource` / `jwks_file` /
`allowed_subjects` 的完整要求见 [`docs/REMOTE_MCP.md`](docs/REMOTE_MCP.md)。
MCP 服务无法弹主机密钥确认框：未知或变更的密钥一律拒绝，所以首次连接前要
预置信任（用上面的 Web 页面点一次“信任并重试”最省事）。

## CLI

```bash
DATA=/var/lib/meatshell
./meatshell --data-dir "$DATA" cli help
./meatshell --data-dir "$DATA" cli import export.json --dry-run
./meatshell --data-dir "$DATA" cli sessions --json
./meatshell --data-dir "$DATA" cli session <session-id>
./meatshell --data-dir "$DATA" cli exec <session-id> -- uptime
./meatshell --data-dir "$DATA" cli files <session-id> /etc
./meatshell --data-dir "$DATA" cli read <session-id> /etc/hosts
./meatshell --data-dir "$DATA" cli upload <session-id> local.txt /tmp
./meatshell --data-dir "$DATA" cli download <session-id> /tmp/a.log ./downloads
```

`--data-dir` 也可用 `MEATSHELL_DATA_DIR`；`--config-info` 打印当前 profile
路径、版本和会话数；`MEATSHELL_LANG=en` 把内置提示切成英文（默认中文）。

## 从源码构建

不需要 GTK / X11 / Wayland，只需要 C 工具链（jemalloc 要编译 C）：

```bash
sudo apt install -y build-essential pkg-config
cargo build --release          # target/release/meatshell
cargo test --bin meatshell     # 216 个单元测试
cargo check --all-targets      # 零警告
```

要求 Rust ≥ 1.88。Docker 构建支持 `CARGO_REGISTRY_MIRROR` 与
`BUILDER_BASE` / `RUNTIME_BASE` 参数，便于国内网络。

## 数据与凭据

所有状态在 profile 目录（容器里是 `/data`，对应宿主机 `./data`）：

| 文件 | 内容 |
| --- | --- |
| `sessions.json` | 会话定义；密码与内联私钥用 ChaCha20-Poly1305 加密（`enc:v1:` 前缀） |
| `secret.key` | profile 主密钥（0600，首次运行生成） |
| `known_hosts` | 主机公钥信任，MeatShell 自有格式 `host:port type base64` |
| `log/` | 运行日志 |

备份 `sessions.json` + `secret.key` 才能完整恢复已存凭据；`secret.key` 丢失后
旧密码无法解密。导出文件里的密码用的是编译进二进制的固定密钥，只算混淆。

## 目录结构

```
src/
  main.rs           入口：cli / mcp / --version
  cli/              CLI 命令实现
  mcp/              stdio + Streamable HTTP MCP、OAuth/JWKS 校验
    impls/webui.rs      连接管理 Web UI（认证、REST API）
    impls/webui.html    嵌入式单页前端（无构建步骤）
  automation/       CLI/MCP/Web 共用的自动化能力（会话、执行、SFTP）
  ssh/              SSH/跳板/代理/PPK/known_hosts、非交互探活
  sftp/             SFTP 传输与提权
  config/           profile、sessions.json、导入导出、加密凭据
  terminal/         仅保留编码适配与 ZMODEM 接收
  tunnel/           端口转发
  logging/          文件日志
docker/             容器部署、令牌工具、Caddy 模板
docs/REMOTE_MCP.md  远程 MCP 部署说明
tests/              Python 端到端回归（HTTP/OAuth、配置导入、跳板链）
```

## 已知限制

- Web UI 的连接测试只支持 SSH；需要 MFA/OTP 的服务器无法在页面完成登录。
- `private_key_path` 按容器内路径解析，用这种方式要额外挂载密钥目录；内联
  私钥没有这个限制。
- 单所有者模型：所有通过校验的 subject 共享同一个 profile。
- 桌面版专属能力（RDP、串口、本地终端、资源面板、单窗口多标签）不在本仓库。
