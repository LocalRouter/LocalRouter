export interface McpAuthConfig {
  type: string
  [key: string]: unknown
}

export type McpAuthMethod = 'none' | 'custom_headers' | 'bearer' | 'oauth_browser' | 'oauth_pregenerated'

export interface OAuthEndpoints {
  auth_url: string
  token_url: string
  scopes_supported: string[]
  grant_types_supported?: string[]
}

/** Undefined preserves auth; empty OAuth secrets preserve an existing OAuth secret. */
export async function buildMcpAuthUpdate(
  method: McpAuthMethod,
  existing: McpAuthConfig | null,
  token: string,
  clientId: string,
  clientSecret: string,
  discover: () => Promise<OAuthEndpoints | null>,
  defaultScopes: string[] = [],
  headers: Record<string, string> = {},
): Promise<McpAuthConfig | undefined> {
  switch (method) {
    case 'none':
      return existing && existing.type !== 'none' ? { type: 'none' } : undefined
    case 'custom_headers': {
      if (Object.keys(headers).length === 0 && existing?.type !== 'custom_headers') throw new Error('Enter at least one custom header')
      const names = new Set<string>()
      for (const [name, value] of Object.entries(headers)) {
        const lower = name.toLowerCase()
        if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name) || names.has(lower)) throw new Error('Enter unique, valid custom header names')
        names.add(lower)
        if (/[\r\n]/.test(value)) throw new Error('Custom header values cannot contain line breaks')
      }
      return { type: 'custom_headers', headers }
    }
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
        scopes: previous?.scopes ?? defaultScopes, redirect_uri: previous?.redirect_uri ?? null,
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
      if (discovery.grant_types_supported?.length && !discovery.grant_types_supported.includes('client_credentials')) {
        throw new Error('This server does not support OAuth client credentials. Use OAuth (Browser login) instead.')
      }
      return {
        type: 'oauth', client_id: id, client_secret: clientSecret,
        auth_url: discovery.auth_url, token_url: discovery.token_url,
        scopes: discovery.scopes_supported,
      }
    }
  }
}


/** Infer legacy transport-header authentication without changing saved configurations. */
export function getMcpAuthMethod(auth: McpAuthConfig | null, headers: Record<string, string> = {}): McpAuthMethod {
  switch (auth?.type) {
    case 'bearer_token': return 'bearer'
    case 'oauth': case 'o_auth': return 'oauth_pregenerated'
    case 'oauth_browser': return 'oauth_browser'
    case 'custom_headers': return 'custom_headers'
    default: return Object.keys(headers).length ? 'custom_headers' : 'none'
  }
}

/** Keep extra transport headers on legacy authenticated servers until auth is explicitly changed. */
export function retainedMcpTransportHeaders(method: McpAuthMethod, auth: McpAuthConfig | null, headers: Record<string, string>): Record<string, string> {
  return method !== 'none' && method !== 'custom_headers' && method === getMcpAuthMethod(auth) ? headers : {}
}
