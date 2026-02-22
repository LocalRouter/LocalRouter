<!-- @entry local-only-design -->

LocalRouter is designed to run entirely on your local machine with no cloud dependencies. The HTTP server binds to `localhost` only — it is not accessible from other machines on the network. All configuration, metrics, and logs are stored locally on disk.

The only network requests LocalRouter makes are those explicitly initiated by the user: API calls to configured LLM providers, connections to configured MCP servers, and optional update checks. There is no "home base" server, no account system, and no cloud sync.

<!-- @entry zero-telemetry -->

LocalRouter contains zero telemetry, analytics, or tracking code. No usage data, crash reports, or diagnostic information is ever sent anywhere. This is enforced as a critical project policy — any telemetry code would be treated as a critical bug.

The application loads no external assets at runtime (fonts, scripts, stylesheets are all bundled at build time), preventing any form of passive tracking through asset loading.

<!-- @entry keychain-storage -->

All sensitive credentials (provider API keys, client secrets, OAuth tokens) are stored in the operating system's native keychain: **macOS Keychain**, **Windows Credential Manager**, or **Linux Secret Service** (GNOME Keyring / KWallet). Secrets are never written to config files, log files, or any other on-disk location.

The keychain entries use the service name `LocalRouter-Providers` for provider keys and `LocalRouter-Clients` for client secrets. For development, set `LOCALROUTER_KEYCHAIN=file` to use a plain text file instead (not recommended for production).

<!-- @entry content-security-policy -->

The Tauri webview enforces a restrictive Content Security Policy (CSP) that blocks external resource loading. Scripts, styles, fonts, and images must all be bundled with the application — no CDN or external URLs are allowed. The CSP also restricts `connect-src` to `localhost` and configured provider domains only.

This prevents XSS attacks, data exfiltration through injected scripts, and accidental loading of external tracking resources.

<!-- @entry open-source-license -->

LocalRouter is licensed under the **GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later)**. This copyleft license ensures the source code remains open and any modifications or derivative works must also be released under the same license.

The AGPL extends the GPL's copyleft provisions to network use — if you run a modified version as a network service, you must make the source code available to users. This protects user freedom and prevents proprietary forks.
