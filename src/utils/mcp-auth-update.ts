export interface McpAuthConfig {
  type: string
  [key: string]: unknown
}

export type McpAuthMethod = 'none' | 'bearer' | 'oauth_browser' | 'oauth_pregenerated'

export interface OAuthEndpoints {
  auth_url: string
  token_url: string
  scopes_supported: string[]
}

/** Undefined preserves auth; empty OAuth secrets preserve an existing OAuth secret. */
export async function buildMcpAuthUpdate(
  method: McpAuthMethod,
  existing: McpAuthConfig | null,
  token: string,
  clientId: string,
  clientSecret: string,
  discover: () => Promise<OAuthEndpoints | null>,
): Promise<McpAuthConfig | undefined> {
  switch (method) {
    case 'none':
      return existing && existing.type !== 'none' ? { type: 'none' } : undefined
    case 'bearer':
      if (token) return { type: 'bearer_token', token }
      if (existing?.type === 'bearer_token') return undefined
      throw new Error('Enter a bearer token to save this authentication method')
    case 'oauth_browser': {
      const previous = existing?.type === 'oauth_browser' ? existing : null
      const id = clientId.trim() || (previous?.client_id as string | undefined)
      if (previous && id === previous.client_id && !clientSecret) return undefined
      return {
        type: 'oauth_browser', client_id: id || null, client_secret: clientSecret || null,
        auth_url: previous?.auth_url ?? null, token_url: previous?.token_url ?? null,
        scopes: previous?.scopes ?? [], redirect_uri: previous?.redirect_uri ?? null,
        issuer: previous?.issuer ?? null,
      }
    }
    case 'oauth_pregenerated': {
      const previous = (existing?.type === 'oauth' || existing?.type === 'o_auth') ? existing : null
      const id = clientId.trim() || (previous?.client_id as string | undefined)
      if (!id || (!clientSecret && !previous)) {
        throw new Error('Enter Client ID and Client Secret to save OAuth credentials')
      }
      const discovery = previous?.auth_url && previous?.token_url ? {
        auth_url: previous.auth_url as string,
        token_url: previous.token_url as string,
        scopes_supported: (previous.scopes as string[] | undefined) ?? [],
      } : await discover()
      if (!discovery) throw new Error('This MCP server does not support OAuth discovery')
      return {
        type: 'oauth', client_id: id, client_secret: clientSecret,
        auth_url: discovery.auth_url, token_url: discovery.token_url,
        scopes: discovery.scopes_supported,
      }
    }
  }
}
