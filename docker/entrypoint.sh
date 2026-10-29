#!/bin/sh
# Entrypoint for the headless MeatShell container.
#
#   no arguments -> start the authenticated MCP HTTP service
#   arguments    -> run a MeatShell command (e.g. `cli sessions --json`)
set -eu

DATA_DIR="${MEATSHELL_DATA_DIR:-/data}"
HTTP_CONFIG="${MEATSHELL_HTTP_CONFIG:-/etc/meatshell/http.json}"

if [ ! -d "$DATA_DIR" ]; then
    echo "meatshell: profile directory $DATA_DIR does not exist" >&2
    exit 1
fi
if [ ! -w "$DATA_DIR" ]; then
    echo "meatshell: profile directory $DATA_DIR is not writable by uid $(id -u)" >&2
    echo "meatshell: chown the host directory to the container user (default 1000:1000)" >&2
    exit 1
fi

has_data_dir=false
for arg in "$@"; do
    if [ "$arg" = "--data-dir" ]; then
        has_data_dir=true
        break
    fi
done

if [ "$#" -gt 0 ]; then
    if [ "$has_data_dir" = true ]; then
        exec /usr/local/bin/meatshell "$@"
    fi
    exec /usr/local/bin/meatshell --data-dir "$DATA_DIR" "$@"
fi

if [ ! -f "$HTTP_CONFIG" ]; then
    echo "meatshell: $HTTP_CONFIG not found" >&2
    echo "meatshell: create config/http.json and config/public-jwks.json (see docker/README.md)" >&2
    exit 1
fi

exec /usr/local/bin/meatshell --data-dir "$DATA_DIR" mcp serve --http-config "$HTTP_CONFIG"
