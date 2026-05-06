#!/usr/bin/env bash
# Container entrypoint for LocalRouter.
#
# On first boot, the app would otherwise default to server.host: 127.0.0.1,
# which is unreachable from outside the container — the user would have to
# stop the app, edit settings.yaml, and restart. We sidestep that by writing
# a minimal settings.yaml before launch when none exists.
#
# All other AppConfig fields are #[serde(default)] (crates/lr-config/src/types.rs:450),
# so the partial file is merged with defaults at load time. ServerConfig
# itself does NOT have field-level serde defaults, so all three fields
# (host, port, enable_cors) must be present in the block we write.

set -euo pipefail

CONFIG_DIR="${HOME}/.localrouter"
CONFIG_FILE="${CONFIG_DIR}/settings.yaml"

if [ ! -f "${CONFIG_FILE}" ]; then
    mkdir -p "${CONFIG_DIR}"
    cat > "${CONFIG_FILE}" <<'YAML'
server:
  host: 0.0.0.0
  port: 3625
  enable_cors: true
YAML
    chmod 600 "${CONFIG_FILE}"
    echo "docker-entrypoint: wrote default ${CONFIG_FILE} (host=0.0.0.0)"
fi

exec "$@"
