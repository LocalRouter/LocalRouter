# MCP server authentication choices

These settings control how LocalRouter connects to a remote MCP server. They are separate from the client API keys used to connect to LocalRouter.

| Choice | Supply | Browser login? |
| --- | --- | --- |
| None (default) | Nothing | No |
| Custom headers | Exact headers required by the server, such as `X-API-Key` | No |
| Bearer Token | An API token or existing OAuth access token | No |
| OAuth (Browser login) | Sign in to the provider; registered app credentials only when required | Yes |
| OAuth (Client credentials — no browser) | An app Client ID and Client Secret; requires `client_credentials` grant support | No |

Browser OAuth may register a client automatically when the server supports it. The generic form hides the optional registered-app fields under **Registered OAuth app (only if required)**. Some services, including Google's Gmail/Drive MCP servers, require your registered OAuth Client ID and Secret as well as browser consent. Their templates request these fields explicitly. A Client Secret identifies the app; it is not an access token and does not replace user consent.

The old **Pre-generated credentials** label meant OAuth client credentials, not a pre-generated user access token. The underlying saved OAuth variant is unchanged for compatibility. Servers supporting only user authorization, such as Google's Workspace setup, cannot use this grant to bypass browser consent. When discovery explicitly rules out `client_credentials`, LocalRouter explains that Browser login is needed rather than starting a browser flow automatically.

Use **Bearer Token** for an access token you already obtained. LocalRouter sends `Authorization: Bearer <token>` and does not refresh pasted tokens. In Browser login mode, LocalRouter obtains tokens and refreshes them when the provider permits it.

Only **Custom headers** displays header inputs. New servers configured as None, Bearer Token or OAuth do not send hidden header drafts. Custom header values are stored in the system keychain. When editing, blank values retain the matching saved secret; remove the row to remove its header. Switching to None explicitly removes the saved authentication.

Existing servers with legacy transport headers keep working. Header-only servers appear as Custom headers, and an explicit configuration edit moves those values into keychain-backed authentication. Additional legacy transport headers on an already authenticated server are retained during unrelated edits; changing its authentication method removes those old transport headers. No background migration changes existing access.

Discovery reports the server's authentication capabilities but does not change your selected method. Choose Browser login explicitly to start user authorization. STDIO servers continue using their environment variables.

Sources:
- MCP authorization and client registration: https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization
- Google's registered-app requirements: https://developers.google.com/workspace/gmail/api/guides/configure-mcp-server
