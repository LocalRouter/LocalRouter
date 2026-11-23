<!-- @entry pp-overview -->

This privacy policy explains what data LocalRouter — the open-source desktop application and this website — collects, how it is used, and how it is shared.

**In short:** LocalRouter runs on your own computer. Its authors do not operate any server that receives your data, and they collect nothing from the application: no account, no telemetry, no analytics, no crash reports.

<!-- @entry pp-data-collected -->

LocalRouter's authors do **not** collect, receive, or store any personal data from the application.

The application itself stores the following **only on your computer**, in its local configuration directory (`~/.localrouter/`) and in your operating system's keychain:

- **Configuration** — the providers, MCP servers, clients, and settings you set up.
- **Credentials** — API keys and OAuth access and refresh tokens for the services you connect. These are kept in the OS keychain (macOS Keychain, Windows Credential Manager, or Linux Secret Service), never in plain config files.
- **Usage metrics and logs** — request counts, token usage, costs, and diagnostic logs, used to show you dashboards and help you troubleshoot.
- **Optional features you enable** — for example conversation memory or response caches.

You can delete all of this at any time by removing the connections in the app, uninstalling it, and deleting its configuration directory.

<!-- @entry pp-data-use -->

LocalRouter uses data only to provide the features you invoke: routing your requests to the AI providers you choose, connecting your AI tools to the MCP servers you configure, and showing you your own usage. It does not use your data for advertising, profiling, or training models, and it does not sell data.

<!-- @entry pp-sharing -->

LocalRouter sends data only to the services **you** configure, at the moment you or an AI tool you connected makes a request — for example a prompt sent to an LLM provider, or a tool call sent to an MCP server. Each such service receives only what that request contains and handles it under its own privacy policy.

No data is sent to LocalRouter's authors. The only other network requests the application makes are optional update checks, which download release information and do not include personal data.

<!-- @entry pp-google-data -->

LocalRouter can connect to Google services such as Gmail through Google's OAuth sign-in. When you connect a Google account:

- **Data accessed:** the Gmail data covered by the permissions you grant — messages, threads, drafts, labels, and settings — and your basic account identity.
- **How it is used:** only to carry out the actions you or the AI tools you connect request through LocalRouter, such as searching, reading, labeling, drafting, or sending email. Requests go directly from your computer to Google.
- **Storage:** OAuth tokens are stored in your operating system's keychain on your computer. Gmail content is not stored by LocalRouter beyond what is needed to complete a request, apart from local logs or caches you enable, which stay on your computer.
- **Sharing:** Google user data is never sent to LocalRouter's authors. It is passed only to the AI tools and model providers that you yourself connect and direct to use it, as part of the request you make. It is never sold, used for advertising, or used to train AI models.
- **Revoking access and deletion:** remove the connection in LocalRouter to delete the stored tokens, and revoke access at any time from your [Google Account permissions](https://myaccount.google.com/permissions).

LocalRouter's use and transfer of information received from Google APIs adheres to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the Limited Use requirements.

<!-- @entry pp-website -->

This website is a static site hosted on GitHub Pages. It sets no cookies and contains no analytics, tracking scripts, or third-party embeds. GitHub, as the host, may process standard request data such as IP addresses for security and operations under the [GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

<!-- @entry pp-security -->

Credentials are kept in the OS keychain, the local API server listens only on `localhost`, and the desktop app enforces a restrictive Content Security Policy. The full source code is public, so anyone can audit exactly what the application does. See [Privacy & Security](/docs/privacy-security) for details.

<!-- @entry pp-children -->

LocalRouter is not directed at children under 13, and its authors knowingly collect no data from anyone.

<!-- @entry pp-changes -->

Changes to this policy are published on this page. Questions can be raised as an issue in the [LocalRouter repository](https://github.com/LocalRouter/LocalRouter/issues). See also the [Terms of Service](/docs/terms-of-service).

Last updated: November 23, 2026.
