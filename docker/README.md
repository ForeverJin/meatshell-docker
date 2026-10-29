# 部署与运行

镜像里只有 headless 二进制：`meatshell cli ...` 和 `meatshell mcp serve`
（stdio / Streamable HTTP + OAuth 2.1 资源服务器）。没有 GUI，也不依赖
X11/Wayland/GTK。

```
MCP 客户端 ──HTTPS──▶ Caddy（可选）──HTTP──▶ meatshell:8765
                                              ├── /data        profile（会话、known_hosts、日志）
                                              └── /etc/meatshell  http.json + public-jwks.json
```

## 1. 目录

| 路径 | 作用 |
| --- | --- |
| `Dockerfile` | 多阶段：`rust:1-bookworm` 编译，`debian:bookworm-slim` 运行 |
| `docker-compose.yml` | 服务编排；`proxy` profile 内含 Caddy |
| `docker/entrypoint.sh` | 无参数启动 MCP HTTP 服务，有参数执行 meatshell 命令 |
| `docker/mint-token.py` | 生成签名密钥/JWKS/http.json，并签发测试或自用 token |
| `docker/http.json.example` | 配置模板 |
| `docker/Caddyfile` | 反向代理模板 |
| `data/` | 运行期 profile（**不要提交**） |
| `config/` | `http.json` + `public-jwks.json`（只读挂载） |
| `secrets/` | RS256 私钥（**不要提交，也不要挂进容器**） |

## 2. 构建

```bash
docker compose build
# 等价于
docker build --build-arg CARGO_REGISTRY_MIRROR=sparse+https://rsproxy.cn/index/ \
             -t meatshell-docker:0.8.0 .

# 只要二进制：
docker build --target binary --output ./out .
```

本机 crates.io 访问慢时，`.env` 里的 `CARGO_REGISTRY_MIRROR` 换成可用镜像即可；
留空则用 crates.io。docker.io 不可用（例如被镜像站 401 拦截）时，用
`BUILDER_BASE` / `RUNTIME_BASE` 指向可用的 registry 副本，例如
`docker.m.daocloud.io/library/rust:1-bookworm`。默认值分别是
`rust:1-bookworm` 和 `debian:bookworm-slim`。

## 3. 初始化

```bash
mkdir -p data && chmod 700 data
python3 docker/mint-token.py init \
  --config-dir config --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ \
  --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang
```

会生成三样东西（已存在时拒绝覆盖）：

- `secrets/jwt-key.pem`：RS256 私钥，权限 600
- `config/public-jwks.json`：对应公钥（只有 `kty/kid/alg/use/n/e`）
- `config/http.json`：`bind`、issuer、resource、required scope、允许的 subject

关于会话兼容性：`"unknown_session"` 控制服务端不认识请求里的
`Mcp-Session-Id` 时的行为。`"reject"`（默认）按 MCP 规范回 404；
`"stateless"` 表示把这一次请求按无状态处理，客户端在服务重启后继续重放旧 ID
也不会卡死（`tools/list`、`tools/call` 照常工作）。代价是这类回退请求不能被
另一个 `notifications/cancelled` 取消，仍受 300 秒上限约束。建议填
`"stateless"`。

关于 `resource` 和 Host 校验：服务要求请求的 `Host` 等于 `resource` 的
authority 或 `bind` 地址。因此**局域网直连**时 `resource` 必须带端口
（`https://192.168.8.114:8765/mcp`）；走 Caddy 时改成
`https://你的域名/mcp` 并把 `MEATSHELL_PUBLISH` 收回 `127.0.0.1`。

`data/` 的属主必须是容器用户（默认 1000:1000），否则启动直接报错。

## 4. 启动

```bash
# .env: MEATSHELL_PUBLISH=0.0.0.0 表示对局域网开放（仅测试用，务必防火墙限制）
docker compose up -d
docker compose ps
docker compose logs -f meatshell
```

容器特性：非 root、`read_only` 根文件系统、`cap_drop: ALL`、
`no-new-privileges`，仅 `/data` 可写，`/tmp` 为 tmpfs，SIGTERM 优雅退出。

## 5. 验证

```bash
# 健康检查（容器内部探测 127.0.0.1:8765 的公开元数据）
docker inspect --format '{{.State.Health.Status}}' meatshell-mcp

# 从局域网另一台机器访问（不要用 127.0.0.1 代替真实地址）
curl -sS http://192.168.8.114:8765/.well-known/oauth-protected-resource/mcp | jq
curl -si -X POST http://192.168.8.114:8765/mcp \
  -H 'Content-Type: application/json' -d '{}' | head -5   # 期望 401

# 签发 token 并初始化会话
TOKEN=$(python3 docker/mint-token.py mint \
  --key secrets/jwt-key.pem \
  --issuer https://meatshell.local/ \
  --resource https://192.168.8.114:8765/mcp \
  --subject mrqiang --ttl 300)
curl -si -X POST http://192.168.8.114:8765/mcp \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual","version":"1"}}}'
```

token 的 `exp` 可以设得比 300 秒长（例如 `--ttl 86400`）；服务对**单次工具
调用**的上限固定为 300 秒或 token 到期，取二者中较早者。`--ttl 0` 表示不过期
（`exp` 写成 9999-12-31），适合给固定客户端长期使用——但 token 一旦泄露就等于
长期授权，当前版本也不支持吊销单个 token，只能换密钥并重启。

`mint-token.py` 只适合自用/自动化：它直接用私钥签 token，跳过了 OAuth 授权码
流程。ChatGPT 之类需要完整 OAuth 2.1（authorization code + PKCE）的客户端，
必须接入真实 IdP，并把 `issuer` / `resource` / `jwks_file` 换成该 IdP 的配置，
见 [`docs/REMOTE_MCP.md`](../docs/REMOTE_MCP.md)。

## 6. Web 管理界面

服务同时在同一个端口提供连接管理页面：`http://192.168.8.114:8765/`。

- 登录：粘贴一个有效的 Bearer token（就是 MCP 用的那个，永久 token 也可以）。
  服务用 `http.json` 里同一套 OAuth 配置校验，成功后下发 HttpOnly +
  SameSite=Strict 的会话 cookie，token 不会存进浏览器。
- 功能：会话列表（搜索、分组）、新建/编辑/删除、测试连接、主机密钥指纹确认。
  测试连接时如果遇到未知主机密钥，页面会显示 SHA256 指纹；核对服务器控制台后点
  “信任并重试”，密钥写入 `data/known_hosts`，与手工预置格式一致。
- 凭据只写不回读：页面只能看到“已存密码/已存密钥”的标记，API 不返回密码或
  私钥内容；编辑时留空表示保持原值，勾选“清除”才会删除。
- 测试连接只支持 SSH，且要求用户名/密码或私钥已经保存（headless 服务无法弹框
  询问）；需要二次验证（MFA/OTP）的服务器无法在页面里完成登录。
- 私钥路径按**容器内**路径解析；用 `private_key_path` 时要额外挂载，例如
  `-v "$PWD/keys:/keys:ro"`，然后填 `/keys/id_ed25519`。内联私钥没有这个限制。
- 不想要页面时在 `config/http.json` 里设 `"web_ui": false` 后重启。

安全边界：能登录页面等价于拿到该 profile 的完整操作权（可增删连接、可让服务用
已存凭据连 SSH），所以 token 要当管理员密码保管。页面启用了 CSP、禁止被 iframe
嵌入，写操作要求自定义头 `X-Ms-Web` 且校验 Origin，跨站请求无法直接调用 API。

## 6. 接入已有会话

```bash
# 预览导入（不写入）
docker compose run --rm -v "$PWD/import:/import:ro" meatshell \
  cli import /import/sessions.json --dry-run
# 正式导入
docker compose run --rm -v "$PWD/import:/import:ro" meatshell \
  cli import /import/sessions.json

docker compose run --rm meatshell cli sessions --json
docker compose run --rm meatshell cli exec <session-id> -- uptime
```

导入文件**不包含** SSH 主机信任。MCP 服务无法弹确认框，未知或变更的主机密钥
一律拒绝，所以首次连接前要把已验证的主机公钥按 MeatShell 自有格式写进
`data/known_hosts`：

```text
<保存的会话 host>:<port> <key-type> <base64-public-host-key>
# 例如
192.168.8.10:22 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...
```

跳板机和最终目标都要各写一条；主机字符串必须与保存的会话完全一致。详见
[`docs/REMOTE_MCP.md`](../docs/REMOTE_MCP.md) 第 1 节。

## 7. HTTPS 反向代理

```bash
# .env
MEATSHELL_PUBLISH=127.0.0.1
MEATSHELL_DOMAIN=shell.example.com

docker compose --profile proxy up -d
```

前提：域名解析到本机、80/443 可达；`config/http.json` 的 `resource` 已改为
`https://shell.example.com/mcp`，并重新签发对应 `aud` 的 token。证书保存在
`caddy/data`，备份时一并保留。Caddyfile 已按官方建议开启
`enable_full_duplex`、`flush_interval -1` 和读写超时。

## 8. 运维

```bash
docker compose logs --tail 100 -f
docker compose restart            # 改 http.json / JWKS 后
docker compose down               # 数据保留在 ./data ./config ./secrets
docker compose build && docker compose up -d   # 升级
```

备份：`./data`（会话、known_hosts、日志）、`./config`、`./secrets`；
启用 Caddy 时再加 `./caddy`。私钥丢失就只能重新 `init` 并重启服务。

## 9. 已知限制

- 服务只校验 token，不签发 token，也不提供登录页或授权码流程。
- 应用只说 HTTP，TLS 必须在反向代理终止；不要直接把 8765 暴露到公网。
- healthcheck 固定探测容器内 `127.0.0.1:8765`；改 `bind` 端口时同步改
  `Dockerfile` 的 `HEALTHCHECK`。
- 这是单所有者服务：所有通过校验的 subject 共享同一个 profile 与文件权限。
