# MCP OAuth Browser Authentication - Setup Guide

## Overview

LocalRouter AI now supports browser-based OAuth authentication for MCP servers, enabling secure integration with services like GitHub, GitLab, and other OAuth-enabled MCP servers.

## Features

✅ **PKCE (S256)** - Industry-standard security for OAuth flows
✅ **Browser-based** - User-friendly authentication in your default browser
✅ **Auto-discovery** - Automatic endpoint detection from `.well-known` paths
✅ **Secure storage** - All credentials stored in OS keychain
✅ **Quick templates** - Pre-configured setups for popular services

---

## Quick Start with Templates

### Creating a GitHub MCP Server

1. **Click "Create Server"** in the MCP Servers tab

2. **Select GitHub Template** 🐙
   - Click the GitHub card in the templates section
   - Form auto-populates with GitHub configuration

3. **Create OAuth App in GitHub**
   - Go to https://github.com/settings/developers
   - Click "OAuth Apps" → "New OAuth App"
   - Fill in:
     - **Application name**: LocalRouter AI - GitHub MCP
     - **Homepage URL**: http://localhost:3625
     - **Authorization callback URL**: `http://localhost:8080/callback` ⚠️
   - Click "Register application"
   - Copy **Client ID** and generate **Client Secret**

4. **Save Server Configuration**
   - Click "Save Changes" in LocalRouter
   - Server is created (but not yet authenticated)

5. **Configure OAuth Credentials**
   - Server will appear in MCP Servers list
   - Click on the server to open detail page
   - Go to **Configuration** tab
   - In "OAuth Authentication Status" section:
     - Paste **Client ID**
     - Paste **Client Secret**
   - Save configuration

6. **Authenticate**
   - Click **"Authenticate"** button
   - Browser window opens automatically
   - Log in to GitHub
   - Grant permissions
   - Return to LocalRouter
   - ✅ Status shows "Authenticated"

7. **Enable and Test**
   - Toggle "Enabled" switch
   - Server starts automatically
   - Go to **Try** tab to test tools

---

## Setting Up GitLab MCP Server

### 1. Create GitLab Application

1. Go to https://gitlab.com/-/profile/applications
2. Fill in:
   - **Name**: LocalRouter AI - GitLab MCP
   - **Redirect URI**: `http://localhost:8080/callback` ⚠️
   - **Scopes**: Select `api`, `read_user`
3. Click "Save application"
4. Copy **Application ID** (Client ID) and **Secret**

### 2. Create Server in LocalRouter

1. Open LocalRouter → MCP Servers tab
2. Click "Create Server"
3. Select **GitLab Template** 🦊
4. Server details auto-populate
5. Click "Save Changes"

### 3. Configure OAuth

1. Click on the GitLab server
2. Go to **Configuration** tab
3. Paste **Application ID** as Client ID
4. Paste **Secret** as Client Secret
5. Save configuration

### 4. Authenticate

1. Click **"Authenticate"** button
2. Browser opens to GitLab
3. Click "Authorize"
4. ✅ Authenticated!

---

## Manual OAuth Browser Setup

If you don't use a template or need custom configuration:

### 1. Create Server

```
Server Name: My OAuth Server
Transport: SSE (Server-Sent Events)
URL: https://api.example.com/mcp
Auth Method: OAuth (Browser Flow)
```

### 2. OAuth Configuration Fields

After server creation, configure in detail page:

- **Client ID**: Your OAuth app client ID
- **Client Secret**: Your OAuth app client secret (stored in keychain)
- **Authorization URL**: Auto-discovered or manual entry
- **Token URL**: Auto-discovered or manual entry
- **Scopes**: Space-separated (e.g., `repo read:user`)
- **Redirect URI**: `http://localhost:8080/callback` (default)

### 3. OAuth Endpoint Discovery

LocalRouter automatically discovers OAuth endpoints from:
```
https://api.example.com/.well-known/oauth-protected-resource
```

If auto-discovery fails, you can manually enter the URLs.

---

## Authentication Management

### Check Authentication Status

**Configuration Tab:**
- Green badge: ✅ Authenticated
- Yellow badge: ⚠️ Not Authenticated

**Actions:**
- **Authenticate** - Start browser OAuth flow
- **Re-authenticate** - Refresh expired token
- **Test Connection** - Verify token is valid
- **Revoke Access** - Remove all OAuth tokens

### Token Expiration

- Tokens are checked before each request
- Auto-refresh supported (if refresh token available)
- Manual re-authentication if refresh fails

---

## Troubleshooting

### "Browser didn't open"
- **Solution**: Click "Open Browser" button in the modal
- **Cause**: Browser blocking or system restrictions

### "Redirect URI mismatch"
- **Error**: OAuth provider rejects callback
- **Solution**: Ensure redirect URI is exactly `http://localhost:8080/callback`
- **Note**: Case-sensitive, no trailing slash

### "Authentication timeout"
- **Cause**: Didn't complete OAuth in 5 minutes
- **Solution**: Click "Authenticate" again

### "Token not found"
- **Cause**: Server started before authentication
- **Solution**:
  1. Stop server
  2. Click "Authenticate"
  3. Complete OAuth flow
  4. Start server

### "Connection test failed"
- **Cause**: Token expired or revoked
- **Solution**: Click "Re-authenticate"

---

## Security Best Practices

### ✅ DO:
- Create separate OAuth apps for development and production
- Use minimal required scopes
- Revoke access when no longer needed
- Keep Client Secret confidential
- Review OAuth app permissions regularly

### ❌ DON'T:
- Share Client Secret in config files
- Commit OAuth credentials to git
- Use production OAuth apps for testing
- Grant excessive scopes

---

## Supported OAuth Providers

### Fully Tested:
- ✅ GitHub
- ✅ GitLab

### Should Work (OAuth 2.0 compliant):
- Bitbucket
- Azure DevOps
- Generic OAuth 2.0 providers

### Requirements:
- OAuth 2.0 authorization code flow
- PKCE (S256) support recommended
- Standard token endpoint
- Callback URL support

---

## Advanced Configuration

### Custom Redirect URI

If port 8080 is in use:

1. Change redirect URI in server config
2. Update OAuth app settings to match
3. LocalRouter will use specified port

**Example:**
```
Redirect URI: http://localhost:9090/callback
```

### Multiple Servers Same Provider

You can create multiple servers for the same OAuth provider:

```
Server 1: GitHub - Personal
  Client ID: <personal-oauth-app>

Server 2: GitHub - Work
  Client ID: <work-oauth-app>
```

Each server maintains separate authentication.

### Offline Mode

OAuth browser authentication requires:
- Active internet connection
- Access to OAuth provider
- Browser with JavaScript enabled

Cannot use offline.

---

## API Reference

### Tauri Commands

Available from frontend:

```typescript
// Start OAuth flow
await invoke('start_mcp_oauth_browser_flow', { serverId })

// Poll status (every 2s)
await invoke('poll_mcp_oauth_browser_status', { serverId })

// Cancel flow
await invoke('cancel_mcp_oauth_browser_flow', { serverId })

// Discover endpoints
await invoke('discover_mcp_oauth_endpoints', { baseUrl })

// Test connection
await invoke('test_mcp_oauth_connection', { serverId })

// Revoke tokens
await invoke('revoke_mcp_oauth_tokens', { serverId })
```

---

## FAQ

**Q: Can I use GitHub Personal Access Tokens instead?**
A: Yes! Use "Bearer Token" auth method instead of OAuth browser flow.

**Q: What happens if I revoke OAuth access?**
A: Server stops working until you re-authenticate. No data is lost.

**Q: Are OAuth tokens encrypted?**
A: Yes, stored in OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service).

**Q: Can I authenticate multiple users?**
A: One authentication per server. Create separate servers for different users.

**Q: How long do tokens last?**
A: Varies by provider. GitHub: 1 year, GitLab: 2 hours (with refresh).

**Q: Can I export OAuth credentials?**
A: No, credentials are tied to OS keychain and cannot be exported for security.

---

## Support

For issues or questions:
- GitHub Issues: https://github.com/anthropics/localrouter-ai/issues
- Documentation: https://localrouter.ai/docs

---

## Change Log

### v0.2.0 (2026-01-21)
- ✅ Initial OAuth browser authentication release
- ✅ GitHub and GitLab templates
- ✅ Auto-endpoint discovery
- ✅ PKCE (S256) security
- ✅ Secure keychain storage

---

**Happy integrating! 🚀**
