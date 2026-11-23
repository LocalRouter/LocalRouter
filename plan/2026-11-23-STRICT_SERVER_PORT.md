# Strict server port

Prompted by [issue #34](https://github.com/LocalRouter/LocalRouter/issues/34): a Windows user
got `Invalid API key` with a freshly copied key. LocalRouter's port 3625 was busy, so the server
moved to 3627. Clients still configured for 3625 (tray Copy URL, Try it out, Sync config all used
the configured port) reached whatever held 3625 — typically another LocalRouter with different
client keys.

## Behaviour

- [x] The server binds only `server.host:server.port`. No next-port fallback. Port 0 still asks
      the OS for an ephemeral port (tests).
- [x] A bind failure is a `ServerBindError` with a user-facing message: in use, access denied
      (Windows reports some in-use or reserved ports this way), or other.
- [x] App startup survives a bind failure: the app runs with the server stopped, and everything
      that wires into the server state uses the state built by the failed start
      (`ServerManager::get_built_state`), so a later start serves it fully wired.
- [x] Status shows the failure: `ServerManager::get_start_error`, `get_server_config.start_error`,
      health cache `server_error`; sidebar shows "Port N unavailable" with the error and a
      Change port link; tray header "LocalRouter: port N unavailable"; the server settings show
      a destructive alert; Save/Restart toast the error instead of claiming success.
- [x] Changing the host, port or CORS (UI or settings file) restarts the server on the new
      address. Starts are serialized so overlapping restarts cannot race for the port.
- [x] Network settings (one global address; there is a single listener) move from
      Clients → Settings to Settings → General. The Clients page keeps only its client list.

## Validation

- [x] Integration test `server_port_in_use_tests`: taken port fails with `port_in_use`, no state
      served, error recorded; a free port then starts and clears it.
- [x] Health cache unit test for recording and clearing the start error.
- [x] Frontend unit test updated: only an OS-assigned port is explained.
- [x] Live, isolated dev instance (`LOCALROUTER_ENV=porttest`) next to production on 3625:
      editing the port to 3625 restarted and failed with the in-use error, nothing else bound;
      editing to 3627 restarted on exactly 3627 (`/health` 200); launching with 3625 kept the app
      up with the server stopped and the error recorded.
- [x] Integration fixtures that derived a fixed port from the PID (and relied on the fallback
      when two tests in one binary collided) now ask for port 0.
- [x] fmt, clippy `--workspace --all-targets -D warnings`, `tsc`, unit tests, app build,
      website `build:check`, targeted Rust suites.

## Review

- Plan review: all behaviour above is implemented.
- Coverage: bind error path, recovery, health cache, URL helper. The UI alert/sidebar paths
  were exercised through the live run's backend state, not with UI automation.
- Bug hunt: startup no longer aborts on bind failure; `stop()` from a failed state keeps the
  error (the server is still not running); a save with an unchanged address does not restart
  (Restart / Start server retry instead).
