import { useEffect, useId, useState } from 'react'
import { Input } from '@/components/ui/Input'
import LegacySelect from '@/components/ui/Select'
import KeyValueInput from '@/components/ui/KeyValueInput'
import type { McpAuthMethod } from '@/utils/mcp-auth-update'

interface Props {
  method: McpAuthMethod
  onMethodChange: (method: McpAuthMethod) => void
  token: string
  onTokenChange: (token: string) => void
  clientId: string
  onClientIdChange: (id: string) => void
  clientSecret: string
  onClientSecretChange: (secret: string) => void
  headers: Record<string, string>
  onHeadersChange: (headers: Record<string, string>) => void
  editing?: boolean
}

/** Shared choices for custom creation, marketplace creation, and saved settings. */
export function McpAuthFields(props: Props) {
  const id = useId()
  const [showClient, setShowClient] = useState(!!props.clientId)
  useEffect(() => { setShowClient(!!props.clientId) }, [props.method])
  const clientFields = (
    <div className="space-y-3 mt-3">
      <div>
        <label htmlFor={`${id}-client`} className="block text-sm font-medium mb-2">Client ID</label>
        <Input id={`${id}-client`} value={props.clientId} onChange={e => props.onClientIdChange(e.target.value)} placeholder="your-oauth-client-id" required={props.method === 'oauth_pregenerated' && !props.editing} />
      </div>
      <div>
        <label htmlFor={`${id}-secret`} className="block text-sm font-medium mb-2">Client Secret</label>
        <Input id={`${id}-secret`} type="password" value={props.clientSecret} onChange={e => props.onClientSecretChange(e.target.value)} placeholder={props.editing ? 'Enter new secret to update (leave empty to keep existing)' : 'your-oauth-client-secret'} required={props.method === 'oauth_pregenerated' && !props.editing} />
        <p className="text-xs text-muted-foreground mt-1">{props.editing ? 'Leave blank to keep the saved secret. ' : ''}{props.method === 'oauth_browser' ? 'Only supply a secret if the server requires one. ' : ''}Credentials are stored in your system keychain.</p>
      </div>
    </div>
  )
  return (
    <div className="space-y-3">
      <div>
        <label htmlFor={`${id}-method`} className="block text-sm font-medium mb-2">Authentication Method</label>
        <LegacySelect id={`${id}-method`} value={props.method} onChange={e => props.onMethodChange(e.target.value as McpAuthMethod)}>
          <option value="none">None</option>
          <option value="custom_headers">Custom headers</option>
          <option value="bearer">Bearer Token</option>
          <option value="oauth_browser">OAuth (Browser login)</option>
          <option value="oauth_pregenerated">OAuth (Client credentials — no browser)</option>
        </LegacySelect>
      </div>
      {props.method === 'none' && <p className="text-xs text-muted-foreground">Connect without authentication.</p>}
      {props.method === 'bearer' && (
        <div>
          <label htmlFor={`${id}-token`} className="block text-sm font-medium mb-2">Bearer Token</label>
          <Input id={`${id}-token`} type="password" value={props.token} onChange={e => props.onTokenChange(e.target.value)} placeholder={props.editing ? 'Enter new token to update (leave empty to keep existing)' : 'your-bearer-token'} required={!props.editing} />
          <p className="text-xs text-muted-foreground mt-1">Use an API token or an OAuth access token you already have. No browser login. LocalRouter does not refresh pasted tokens.{props.editing ? ' Leave blank to keep the saved token.' : ''}</p>
        </div>
      )}
      {props.method === 'oauth_browser' && (
        <div>
          <p className="text-xs text-muted-foreground">Sign in and grant access in your browser. Leave app credentials empty when the server supports automatic client registration. Servers such as Google require a registered Client ID and Client Secret.</p>
          <details open={showClient} onToggle={e => setShowClient(e.currentTarget.open)}>
            <summary className="cursor-pointer text-sm mt-2">Registered OAuth app (only if required)</summary>
            {clientFields}
          </details>
        </div>
      )}
      {props.method === 'oauth_pregenerated' && (
        <div>
          <p className="text-xs text-muted-foreground">App-to-app OAuth: LocalRouter exchanges a Client ID and Client Secret for access tokens automatically. No browser login. The server must support the client_credentials grant. For Gmail or Drive, use Browser login. For an existing access token, use Bearer Token.</p>
          {clientFields}
        </div>
      )}
      {props.method === 'custom_headers' && (
        <div>
          <p className="text-xs text-muted-foreground mb-2">Enter the exact headers required by the server, such as X-API-Key. Values are stored in your system keychain.{props.editing ? ' Blank values keep existing saved headers; remove a row to remove that header.' : ''}</p>
          <KeyValueInput value={props.headers} onChange={props.onHeadersChange} keyPlaceholder="Header Name" valuePlaceholder="Header Value" secretValue />
        </div>
      )}
    </div>
  )
}
