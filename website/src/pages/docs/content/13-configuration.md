<!-- @entry yaml-config -->

LocalRouter uses a YAML configuration file as its primary configuration store. The config file defines providers (type and enabled status — keys are stored separately in the keychain), clients (name, strategy reference, permissions), strategies (model selection, rate limits), and MCP server configurations (transport, auth, tools).

The config is loaded on startup and watched for changes. Modifications through the UI are written back to the YAML file automatically.

<!-- @entry config-file-location -->

The configuration file location depends on the operating system and whether LocalRouter is running in development or production mode.

<!-- @entry config-macos -->

**Production**: `~/Library/Application Support/LocalRouter/config.yaml`
**Development**: `~/.localrouter-dev/config.yaml`

The Application Support directory is created automatically on first launch if it doesn't exist.

<!-- @entry config-linux -->

**Production**: `~/.localrouter/config.yaml`
**Development**: `~/.localrouter-dev/config.yaml`

Follows XDG conventions when `XDG_CONFIG_HOME` is set, falling back to `~/.localrouter/`.

<!-- @entry config-windows -->

**Production**: `%APPDATA%\LocalRouter\config.yaml`
**Development**: `~/.localrouter-dev/config.yaml`

The `%APPDATA%` directory typically resolves to `C:\Users\<username>\AppData\Roaming`.

<!-- @entry config-migration -->

When the config format changes between versions, LocalRouter automatically migrates your config on startup. A backup of the pre-migration config is saved before any changes are applied, so your data is safe even if something goes wrong.

<!-- @entry environment-variables -->

Several environment variables override config file settings:

- `LOCALROUTER_PORT` — Override the HTTP server port (default: 3625)
- `LOCALROUTER_KEYCHAIN` — Set to `file` to store secrets in a plain text file instead of the OS keychain (development only, not recommended for production)
- `LOCALROUTER_LOG_LEVEL` — Set logging verbosity (`trace`, `debug`, `info`, `warn`, `error`)
- `LOCALROUTER_DATA_DIR` — Override the data directory path
- `LOCALROUTER_CONFIG` — Override the config file path

Environment variables take precedence over config file values but do not modify the config file.
