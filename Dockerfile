# syntax=docker/dockerfile:1.7
#
# Headless MeatShell container: CLI and authenticated MCP HTTP service.
# There is no desktop UI in this tree, so neither the build nor the runtime
# image needs X11/Wayland/GTK/font packages.

# Base images are parameters so a host behind a registry mirror can point them
# at a working copy without editing this file.
ARG BUILDER_BASE=rust:1-bookworm
ARG RUNTIME_BASE=debian:bookworm-slim

FROM ${BUILDER_BASE} AS builder

ARG CARGO_BUILD_JOBS=4
# Optional crates.io mirror for slow networks, e.g.
#   --build-arg CARGO_REGISTRY_MIRROR=sparse+https://rsproxy.cn/index/
ARG CARGO_REGISTRY_MIRROR=""

ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} \
    CARGO_TERM_COLOR=always \
    CARGO_NET_RETRY=5

RUN if [ -n "${CARGO_REGISTRY_MIRROR}" ]; then \
        printf '[source.crates-io]\nreplace-with = "mirror"\n\n[source.mirror]\nregistry = "%s"\n' \
            "${CARGO_REGISTRY_MIRROR}" > /usr/local/cargo/config.toml; \
    fi

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release --bin meatshell \
    && install -Dm0755 target/release/meatshell /usr/local/bin/meatshell

# `docker build --target binary --output ./out .` exports just the binary.
FROM scratch AS binary
COPY --from=builder /usr/local/bin/meatshell /meatshell

FROM ${RUNTIME_BASE} AS runtime

# The binary links only libc, libgcc and libm. ca-certificates is needed for
# outbound TLS (HTTPS proxies); curl backs the healthcheck; tzdata makes TZ work.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl tzdata \
    && rm -rf /var/lib/apt/lists/*

# Never run as root. Default to the usual NAS first user; override with build
# args when the host profile directory belongs to someone else.
ARG MEATSHELL_UID=1000
ARG MEATSHELL_GID=1000
RUN groupadd -g "${MEATSHELL_GID}" meatshell \
    && useradd -u "${MEATSHELL_UID}" -g meatshell -d /data -s /usr/sbin/nologin meatshell \
    && install -d -m 0700 -o meatshell -g meatshell /data \
    && install -d -m 0755 -o root -g root /etc/meatshell

COPY --from=builder /usr/local/bin/meatshell /usr/local/bin/meatshell
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
COPY docker/http.json.example /etc/meatshell/http.json.example
COPY docker/mint-token.py /etc/meatshell/mint-token.py
COPY docs/REMOTE_MCP.md /usr/share/doc/meatshell/REMOTE_MCP.md
RUN chmod 0755 /usr/local/bin/entrypoint.sh /usr/local/bin/meatshell

ENV MEATSHELL_DATA_DIR=/data \
    MEATSHELL_HTTP_CONFIG=/etc/meatshell/http.json

USER meatshell
WORKDIR /data
EXPOSE 8765

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -fsS -o /dev/null "http://127.0.0.1:8765/.well-known/oauth-protected-resource/mcp" || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
