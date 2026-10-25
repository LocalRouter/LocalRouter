import { test, expect } from '@playwright/test'
import { buildMcpAuthUpdate, type McpAuthConfig } from '../../../src/utils/mcp-auth-update'

const endpoints = { auth_url: 'https://example.com/authorize', token_url: 'https://example.com/token', scopes_supported: ['read'] }
const discover = async () => endpoints
const noDiscovery = async () => { throw new Error('Unexpected discovery') }

test('clearing auth sends the explicit none variant rather than the rejected JSON null', async () => {
  expect(await buildMcpAuthUpdate('none', { type: 'o_auth' }, '', '', '', noDiscovery)).toEqual({ type: 'none' })
  expect(await buildMcpAuthUpdate('none', null, '', '', '', noDiscovery)).toBeUndefined()
})

test('new pre-generated OAuth requires both credentials and includes discovered endpoints', async () => {
  for (const [id, secret] of [['', 'secret'], ['client', '']]) {
    await expect(buildMcpAuthUpdate('oauth_pregenerated', null, '', id, secret, noDiscovery)).rejects.toThrow('Client ID and Client Secret')
  }
  expect(await buildMcpAuthUpdate('oauth_pregenerated', null, '', 'client', 'secret', discover)).toEqual({
    type: 'oauth', client_id: 'client', client_secret: 'secret', auth_url: endpoints.auth_url, token_url: endpoints.token_url, scopes: ['read'],
  })
  await expect(buildMcpAuthUpdate('oauth_pregenerated', null, '', 'client', 'secret', async () => null)).rejects.toThrow('discovery')
})

test('both serialized OAuth spellings keep credentials and existing endpoint metadata during partial edits', async () => {
  for (const type of ['o_auth', 'oauth']) {
    const existing = { type, client_id: 'original', client_secret_ref: 'kept', auth_url: endpoints.auth_url, token_url: endpoints.token_url, scopes: ['custom'] }
    const result = await buildMcpAuthUpdate('oauth_pregenerated', existing, '', '', '', noDiscovery)
    expect(result).toMatchObject({ client_id: 'original', client_secret: '', scopes: ['custom'] })
    expect(await buildMcpAuthUpdate('oauth_pregenerated', existing, '', 'replacement', 'new secret', noDiscovery)).toMatchObject({ client_id: 'replacement', client_secret: 'new secret' })
    expect(await buildMcpAuthUpdate('oauth_pregenerated', { ...existing, auth_url: '', token_url: '' }, '', '', '', discover)).toMatchObject({ auth_url: endpoints.auth_url, token_url: endpoints.token_url })
  }
})

test('browser OAuth accepts registered app credentials and retains issuer, scopes, redirect and endpoints', async () => {
  expect(await buildMcpAuthUpdate('oauth_browser', null, '', 'slack-client', 'slack-secret', noDiscovery)).toMatchObject({ type: 'oauth_browser', client_id: 'slack-client', client_secret: 'slack-secret' })
  const existing: McpAuthConfig = { type: 'oauth_browser', client_id: 'client', auth_url: endpoints.auth_url, token_url: endpoints.token_url, scopes: ['custom'], issuer: 'https://example.com', redirect_uri: 'http://localhost:9090/callback' }
  expect(await buildMcpAuthUpdate('oauth_browser', existing, '', '', '', noDiscovery)).toBeUndefined()
  expect(await buildMcpAuthUpdate('oauth_browser', existing, '', 'new-client', '', noDiscovery)).toEqual({ ...existing, client_id: 'new-client', client_secret: null })
})

test('switching auth never reuses credentials from a different method', async () => {
  await expect(buildMcpAuthUpdate('bearer', { type: 'oauth_browser' }, '', '', '', noDiscovery)).rejects.toThrow('bearer token')
  expect(await buildMcpAuthUpdate('bearer', { type: 'bearer_token' }, '', '', '', noDiscovery)).toBeUndefined()
  expect(await buildMcpAuthUpdate('bearer', null, 'new-token', '', '', noDiscovery)).toEqual({ type: 'bearer_token', token: 'new-token' })
  await expect(buildMcpAuthUpdate('oauth_pregenerated', { type: 'oauth_browser', client_id: 'client' }, '', '', '', noDiscovery)).rejects.toThrow('Client ID')
})


test('Google template scopes limit new browser grants and preserve saved scopes on edits', async () => {
  const scopes = ['https://www.googleapis.com/auth/gmail.readonly', 'https://www.googleapis.com/auth/gmail.compose']
  expect(await buildMcpAuthUpdate('oauth_browser', null, '', 'google-client', 'secret', noDiscovery, scopes)).toMatchObject({ scopes })
  expect(await buildMcpAuthUpdate('oauth_browser', { type: 'oauth_browser', client_id: 'old', scopes: ['saved'] }, '', 'new', '', noDiscovery, scopes)).toMatchObject({ scopes: ['saved'] })
})
