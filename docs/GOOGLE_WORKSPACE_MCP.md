# Google-hosted Gmail and Drive MCP servers

LocalRouter connects directly to Google's hosted servers using your own Google Cloud OAuth registration. No LocalRouter-owned Google project or credentials are used. A project identifies your OAuth app and enables the APIs; it does not host an MCP server.

## Requirements

Google currently documents a Google Cloud project and Google Workspace Developer Preview enrollment. Enroll the project through https://developers.google.com/workspace/preview and wait for Google's confirmation. Creating OAuth credentials alone does not establish preview eligibility. The same project and client can cover both services.

## Configure your project

1. Create or select your project at https://console.cloud.google.com/ .
2. Enable the APIs for the services you want:

   | Template | APIs |
   | --- | --- |
   | Gmail | `gmail.googleapis.com`, `gmailmcp.googleapis.com` |
   | Google Drive | `drive.googleapis.com`, `drivemcp.googleapis.com` |

3. In **Google Auth Platform → Branding**, configure your app name, support email and contact email. Review Google's policy yourself before agreeing.
4. Choose **Internal** for use within your Workspace organization. Otherwise choose **External** and, while testing, add your Google account under **Audience → Test users**.
5. Under **Data Access**, add the documented scopes for each service:

   | Template | OAuth scopes |
   | --- | --- |
   | Gmail | `https://www.googleapis.com/auth/gmail.readonly`, `https://www.googleapis.com/auth/gmail.compose` |
   | Google Drive | `https://www.googleapis.com/auth/drive.readonly`, `https://www.googleapis.com/auth/drive.file` |

6. Under **Clients**, create a **Web application** OAuth client. Select **Use this client for an AI-powered agent**. Add **`http://localhost:8080/callback`** as an authorized redirect URI. Use the exact URI, including the port and path; the API port 3625 is not the OAuth callback port.
7. Keep your Client ID and Client Secret for the LocalRouter form. Do not put them in a repository or use the client secret as a bearer token.

## Add the servers in LocalRouter

Select the **Gmail** or **Google Drive** template, enter the registered Client ID and Client Secret, and create the server. Complete browser login with the Google account you want to access.

| Setting | Gmail | Google Drive |
| --- | --- | --- |
| URL | `https://gmailmcp.googleapis.com/mcp/v1` | `https://drivemcp.googleapis.com/mcp/v1` |
| Transport | HTTP / Streamable HTTP | HTTP / Streamable HTTP |
| Authentication | OAuth (Browser login) | OAuth (Browser login) |
| Custom headers | Leave empty | Leave empty |

LocalRouter discovers Google's authorization/token endpoints, stores credentials through its keychain integration, and supplies `Authorization: Bearer <access_token>` automatically. No API key or project-ID header is prescribed by Google's setup guides. Each server has its own user authorization.

For an installed LocalRouter version without these templates, create a custom HTTP server with the same URL and browser OAuth credentials. Check the requested scopes on Google's consent screen: these new templates limit the initial scopes to the documented pairs, whereas older versions may request all scopes advertised by Google's discovery metadata.

## Troubleshooting

- **MCP API unavailable or access denied:** confirm that Google enrolled this exact project in the preview and enabled both the product and MCP APIs. Workspace administrator controls can also restrict your OAuth app.
- **redirect_uri_mismatch:** register `http://localhost:8080/callback` exactly and make sure port 8080 is available locally.
- **Agent/client rejected:** create a client marked **Use this client for an AI-powered agent**; a legacy client may not expose the setting.
- **Repeated login:** Google requires `access_type=offline` for refresh tokens; LocalRouter's updated Google browser flow requests it with `prompt=consent`. An older installed version may require hourly login. External apps with publishing status Testing can also have refresh tokens expire after seven days.
- **Drive file missing:** Google's MCP server has additional file-eligibility rules; success listing tools does not prove that a particular file is available.
- **Gmail operation unavailable:** the documented scope pair supports reading and composing. Other operations can require additional scopes. Grant only the access you intend.

## Sources

- Gmail setup: https://developers.google.com/workspace/gmail/api/guides/configure-mcp-server
- Drive setup and linked file-eligibility guide: https://developers.google.com/workspace/drive/api/guides/configure-mcp-server
- Preview enrollment: https://developers.google.com/workspace/preview
- Google OAuth refresh-token behavior: https://developers.google.com/identity/protocols/oauth2/web-server#offline

Requirements checked 2026-10-24. Preview availability can change.
