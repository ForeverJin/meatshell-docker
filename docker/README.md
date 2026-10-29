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
