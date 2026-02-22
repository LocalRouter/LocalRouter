<!-- @entry installation -->

LocalRouter is distributed as a desktop application for macOS, Windows, and Linux. Download the latest release from the GitHub releases page or build from source with `cargo tauri build --release`.

<!-- @entry install-macos -->

Download the `.dmg` file from the releases page. Open the disk image and drag LocalRouter to your Applications folder. On first launch, macOS may show a security prompt — right-click the app and select "Open" to bypass Gatekeeper. Configuration is stored in `~/Library/Application Support/LocalRouter/` and secrets in the macOS Keychain under the `LocalRouter-Clients` service.

<!-- @entry install-windows -->

Download the `.msi` installer from the releases page and run it. LocalRouter installs to the standard Program Files directory. Configuration is stored in `%APPDATA%\LocalRouter\` and secrets are managed via the Windows Credential Manager.

<!-- @entry install-linux -->

Download the `.AppImage` or `.deb` package from the releases page. For the AppImage, make it executable with `chmod +x` and run directly. For Debian-based systems, install with `sudo dpkg -i localrouter_*.deb`. Configuration is stored in `~/.localrouter/` and secrets are stored using the system's secret service (e.g., GNOME Keyring or KWallet).

<!-- @entry first-run -->

When you first launch LocalRouter, the Axum HTTP server starts on `localhost:3625`. The UI opens in a native window showing the dashboard. No providers are configured yet, so the first step is adding at least one provider API key. You can verify the server is running by visiting `http://localhost:3625/health` in your browser — it should return a healthy status response.

<!-- @entry configuring-first-provider -->

Navigate to the **Resources** view in the UI and add your first provider. Select a provider (e.g., OpenAI, Anthropic), enter your API key, and save. The key is stored securely in your OS keychain — never in config files. Once saved, LocalRouter fetches the provider's available models and adds them to the model catalog. You can now send requests to `localhost:3625` using any of that provider's models.

<!-- @entry pointing-apps -->

Point any OpenAI-compatible application to `http://localhost:3625/v1` as the base URL, and use your LocalRouter client secret (`lr-*`) as the API key. For example, in a Python OpenAI client: `client = OpenAI(base_url="http://localhost:3625/v1", api_key="lr-your_key")`. For Claude Code, Cursor, or other AI tools, update their API base URL setting to `http://localhost:3625/v1`. LocalRouter accepts the standard `Authorization: Bearer <token>` header format that all OpenAI-compatible clients use.
