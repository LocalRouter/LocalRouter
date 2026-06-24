import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-shell'
import { isValidHttpUrl } from '@/utils/url'
import Button from './ui/Button'
import { EyeIcon, EyeSlashIcon } from '@heroicons/react/24/outline'
import type { OAuthCredentialView } from '@/types/tauri-commands'
import { errorMessage } from '@/utils/errors'

/**
 * Maps a provider *type* (as exposed by `list_provider_types`) to the backend
 * OAuth provider ID used by the OAuth commands. Shared so the create form and
 * the settings tab agree on the mapping.
 */
export const OAUTH_PROVIDER_MAP: Record<string, string> = {
  'github-copilot': 'github-copilot',
  'openai-chatgpt-plus': 'openai-codex',
}

interface OAuthFlowResult {
  type: 'pending' | 'success' | 'error'
  user_code?: string
  verification_url?: string
  instructions?: string
  message?: string
}

/**
 * Self-contained OAuth credential management for an already-added provider.
 *
 * Shown in the provider Settings tab so a user whose token expired or was
 * revoked can re-authenticate in place — previously the only recourse was to
 * delete and re-create the provider. It detects whether credentials are
 * currently stored, lets the user inspect/copy the access token, and drives a
 * fresh OAuth flow (browser PKCE or device code) via the existing backend
 * commands.
 */
export function OAuthSettingsControls({
  oauthProviderId,
  displayName,
}: {
  oauthProviderId: string
  displayName: string
}) {
  const [cred, setCred] = useState<OAuthCredentialView | null>(null)
  const [authenticated, setAuthenticated] = useState<boolean | null>(null)
  const [showToken, setShowToken] = useState(false)
  const [status, setStatus] = useState<'idle' | 'pending' | 'error'>('idle')
  const [result, setResult] = useState<OAuthFlowResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Load (or reload) the current credential state from the backend.
  const loadCredential = useCallback(async () => {
    try {
      const providers = await invoke<string[]>('list_oauth_credentials')
      const has = providers.includes(oauthProviderId)
      setAuthenticated(has)
      if (has) {
        const c = await invoke<OAuthCredentialView | null>('get_oauth_token', {
          providerId: oauthProviderId,
        })
        setCred(c)
      } else {
        setCred(null)
      }
    } catch (e) {
      console.error('Failed to load OAuth credential:', e)
      setAuthenticated(false)
    }
  }, [oauthProviderId])

  useEffect(() => {
    loadCredential()
  }, [loadCredential])

  // Poll for completion while a flow is in progress.
  useEffect(() => {
    if (status !== 'pending') return

    let consecutiveErrors = 0
    const pollInterval = setInterval(async () => {
      try {
        const r = await invoke<OAuthFlowResult>('poll_oauth_status', { providerId: oauthProviderId })
        consecutiveErrors = 0
        if (r.type === 'success') {
          setStatus('idle')
          setResult(null)
          clearInterval(pollInterval)
          loadCredential()
        } else if (r.type === 'error') {
          setStatus('error')
          setError(r.message || 'OAuth failed')
          clearInterval(pollInterval)
        }
      } catch (err) {
        consecutiveErrors++
        console.error('OAuth poll error:', err)
        if (consecutiveErrors >= 3) {
          setStatus('error')
          setError(errorMessage(err, 'OAuth poll failed'))
          clearInterval(pollInterval)
        }
      }
    }, 2000)

    return () => clearInterval(pollInterval)
  }, [status, oauthProviderId, loadCredential])

  const startFlow = useCallback(async () => {
    setStatus('pending')
    setError(null)
    setResult(null)
    try {
      const r = await invoke<OAuthFlowResult>('start_oauth_flow', { providerId: oauthProviderId })
      setResult(r)
      if (r.type === 'success') {
        setStatus('idle')
        loadCredential()
      } else if (r.type === 'error') {
        setStatus('error')
        setError(r.message || 'OAuth failed')
      } else if (r.type === 'pending' && r.verification_url) {
        // Browser PKCE flows: auto-open the browser. Device-code flows
        // (which carry a user_code) instead surface the code below.
        if (!r.user_code && isValidHttpUrl(r.verification_url)) {
          try {
            await open(r.verification_url)
          } catch (e) {
            console.error('Failed to open browser:', e)
          }
        }
      }
    } catch (err) {
      console.error('start_oauth_flow failed:', err)
      setStatus('error')
      setError(errorMessage(err, 'Failed to start OAuth flow'))
    }
  }, [oauthProviderId, loadCredential])

  const tokenPreview = cred?.access_token
    ? showToken
      ? cred.access_token
      : `${cred.access_token.slice(0, 12)}…${cred.access_token.slice(-4)}`
    : null

  const expiresAtMs = cred?.expires_at != null ? cred.expires_at * 1000 : null
  const expiresLabel = expiresAtMs != null ? new Date(expiresAtMs).toLocaleString() : null
  const isExpired = expiresAtMs != null && expiresAtMs < Date.now()

  return (
    <div className="space-y-3 p-4 border rounded-lg bg-muted/30">
      <div className="flex items-center justify-between">
        <div>
          <p className="font-medium text-sm">Authentication</p>
          <p className="text-xs text-muted-foreground">
            Sign in with your {displayName} account
          </p>
        </div>
        {authenticated && status === 'idle' && (
          <span
            className={
              isExpired
                ? 'text-xs text-amber-600 dark:text-amber-400 font-medium'
                : 'text-xs text-green-600 dark:text-green-400 font-medium'
            }
          >
            {isExpired ? 'Token expired' : '✓ Connected'}
          </span>
        )}
      </div>

      {/* Connected: show token details + reconnect */}
      {status === 'idle' && authenticated && (
        <div className="p-3 bg-background rounded border space-y-2 text-xs">
          {cred ? (
            <>
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">Access token</span>
                <div className="flex gap-1">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => setShowToken((v) => !v)}
                    title={showToken ? 'Hide token' : 'Show token'}
                  >
                    {showToken ? <EyeSlashIcon className="h-3 w-3" /> : <EyeIcon className="h-3 w-3" />}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => {
                      if (cred.access_token) {
                        navigator.clipboard.writeText(cred.access_token)
                      }
                    }}
                    title="Copy token"
                  >
                    Copy
                  </Button>
                </div>
              </div>
              <p className="font-mono break-all text-muted-foreground">{tokenPreview}</p>
              {expiresLabel && (
                <p className={isExpired ? 'text-amber-600 dark:text-amber-400' : 'text-muted-foreground'}>
                  {isExpired ? 'Expired: ' : 'Expires: '}
                  {expiresLabel}
                </p>
              )}
              {cred.account_id && (
                <p className="text-muted-foreground">Account: {cred.account_id}</p>
              )}
            </>
          ) : (
            <p className="text-muted-foreground">No token details available.</p>
          )}
          <Button type="button" size="sm" variant="secondary" onClick={startFlow}>
            Reconnect
          </Button>
        </div>
      )}

      {/* Not connected: prompt to authenticate */}
      {status === 'idle' && authenticated === false && (
        <Button type="button" onClick={startFlow} variant="secondary">
          Connect with {displayName}
        </Button>
      )}

      {/* Flow in progress */}
      {status === 'pending' && (
        <div className="space-y-2 text-sm">
          {result?.user_code && (
            <>
              <div className="p-3 bg-background rounded border">
                <p className="text-muted-foreground mb-1">Enter this code:</p>
                <p className="font-mono text-lg font-bold">{result.user_code}</p>
              </div>
              {result.verification_url && (
                <p className="text-muted-foreground">
                  Visit:{' '}
                  {isValidHttpUrl(result.verification_url) ? (
                    <a
                      href={result.verification_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-500 hover:underline"
                    >
                      {result.verification_url}
                    </a>
                  ) : (
                    <span>{result.verification_url}</span>
                  )}
                </p>
              )}
            </>
          )}
          {!result?.user_code && (
            <div className="p-3 bg-background rounded border space-y-2">
              <p className="text-muted-foreground text-center">
                A browser window has been opened. If it didn't, use the link below.
              </p>
              {result?.verification_url && isValidHttpUrl(result.verification_url) && (
                <div className="flex flex-col gap-2">
                  <a
                    href={result.verification_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-500 hover:underline text-xs break-all"
                    title="Open in browser"
                  >
                    {result.verification_url}
                  </a>
                  <div className="flex gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => result.verification_url && open(result.verification_url)}
                    >
                      Open in Browser
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => {
                        if (result.verification_url) {
                          navigator.clipboard.writeText(result.verification_url)
                        }
                      }}
                    >
                      Copy Link
                    </Button>
                  </div>
                </div>
              )}
            </div>
          )}
          <p className="text-muted-foreground animate-pulse text-center">
            Waiting for authentication...
          </p>
        </div>
      )}

      {/* Error */}
      {status === 'error' && (
        <div className="space-y-2">
          <p className="text-sm text-red-500">{error}</p>
          <Button type="button" onClick={startFlow} variant="secondary" size="sm">
            Try Again
          </Button>
        </div>
      )}
    </div>
  )
}
