import Input from '../ui/Input'
import Select from '../ui/Select'

export interface McpConfigFormData {
  serverName: string
  transportType: 'Stdio' | 'Sse' | 'WebSocket'
  // STDIO config
  command: string
  args: string
  envVars: string
  // SSE/WebSocket config
  url: string
  headers: string
  // Auth config
  authMethod: 'none' | 'bearer' | 'custom_headers' | 'oauth' | 'env_vars'
  bearerToken: string
  authHeaders: string
  authEnvVars: string
  oauthClientId: string
  oauthClientSecret: string
  oauthAuthUrl: string
  oauthTokenUrl: string
  oauthScopes: string
}

interface McpConfigFormProps {
  formData: McpConfigFormData
  onChange: (field: keyof McpConfigFormData, value: string) => void
  disabled?: boolean
  showTransportType?: boolean
}

export default function McpConfigForm({ formData, onChange, disabled = false, showTransportType = true }: McpConfigFormProps) {
  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium mb-2">
          Server Name
        </label>
        <Input
          value={formData.serverName}
          onChange={(e) => onChange('serverName', e.target.value)}
          placeholder="My MCP Server"
          disabled={disabled}
          required
        />
      </div>

      {showTransportType && (
        <div>
          <label className="block text-sm font-medium mb-2">
            Transport Type
          </label>
          <Select
            value={formData.transportType}
            onChange={(e) => onChange('transportType', e.target.value)}
            disabled={disabled}
          >
            <option value="Stdio">STDIO (Subprocess)</option>
            <option value="Sse">SSE (Server-Sent Events)</option>
            <option value="WebSocket">WebSocket</option>
          </Select>
        </div>
      )}

      {/* STDIO Config */}
      {formData.transportType === 'Stdio' && (
        <>
          <div>
            <label className="block text-sm font-medium mb-2">
              Command
            </label>
            <Input
              value={formData.command}
              onChange={(e) => onChange('command', e.target.value)}
              placeholder="npx"
              disabled={disabled}
              required
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Arguments (one per line)
            </label>
            <textarea
              value={formData.args}
              onChange={(e) => onChange('args', e.target.value)}
              placeholder="-y&#10;@modelcontextprotocol/server-everything"
              className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
              rows={3}
              disabled={disabled}
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Environment Variables (KEY=VALUE, one per line)
            </label>
            <textarea
              value={formData.envVars}
              onChange={(e) => onChange('envVars', e.target.value)}
              placeholder="API_KEY=your_key&#10;DEBUG=true"
              className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
              rows={3}
              disabled={disabled}
            />
          </div>
        </>
      )}

      {/* SSE/WebSocket Config */}
      {(formData.transportType === 'Sse' || formData.transportType === 'WebSocket') && (
        <>
          <div>
            <label className="block text-sm font-medium mb-2">
              URL
            </label>
            <Input
              value={formData.url}
              onChange={(e) => onChange('url', e.target.value)}
              placeholder={formData.transportType === 'WebSocket' ? 'ws://localhost:3000' : 'http://localhost:3000'}
              disabled={disabled}
              required
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Headers (KEY: VALUE, one per line)
            </label>
            <textarea
              value={formData.headers}
              onChange={(e) => onChange('headers', e.target.value)}
              placeholder="Authorization: Bearer token&#10;X-Custom-Header: value"
              className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
              rows={3}
              disabled={disabled}
            />
          </div>
        </>
      )}

      {/* Authentication Configuration */}
      <div className="border-t border-gray-700 pt-4 mt-4">
        <h3 className="text-md font-semibold mb-3">Authentication (Optional)</h3>
        <p className="text-sm text-gray-400 mb-3">
          Configure how LocalRouter authenticates to this MCP server
        </p>

        <div>
          <label className="block text-sm font-medium mb-2">
            Auth Method
          </label>
          <Select
            value={formData.authMethod}
            onChange={(e) => onChange('authMethod', e.target.value)}
            disabled={disabled}
          >
            <option value="none">None (No Authentication)</option>
            {formData.transportType !== 'Stdio' && <option value="bearer">Bearer Token</option>}
            {formData.transportType !== 'Stdio' && <option value="custom_headers">Custom Headers</option>}
            {formData.transportType !== 'Stdio' && <option value="oauth">OAuth (Pre-registered)</option>}
            {formData.transportType === 'Stdio' && <option value="env_vars">Environment Variables</option>}
          </Select>
        </div>

        {/* Bearer Token Auth */}
        {formData.authMethod === 'bearer' && (
          <div className="mt-3">
            <label className="block text-sm font-medium mb-2">
              Bearer Token
            </label>
            <Input
              type="password"
              value={formData.bearerToken}
              onChange={(e) => onChange('bearerToken', e.target.value)}
              placeholder="your-bearer-token"
              disabled={disabled}
              required
            />
            <p className="text-xs text-gray-500 mt-1">
              Token will be stored securely in system keychain
            </p>
          </div>
        )}

        {/* Custom Headers Auth */}
        {formData.authMethod === 'custom_headers' && (
          <div className="mt-3">
            <label className="block text-sm font-medium mb-2">
              Auth Headers (KEY: VALUE, one per line)
            </label>
            <textarea
              value={formData.authHeaders}
              onChange={(e) => onChange('authHeaders', e.target.value)}
              placeholder="Authorization: Bearer token&#10;X-API-Key: your-key"
              className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
              rows={3}
              disabled={disabled}
              required
            />
            <p className="text-xs text-gray-500 mt-1">
              These headers will be sent with every request
            </p>
          </div>
        )}

        {/* OAuth Auth */}
        {formData.authMethod === 'oauth' && (
          <div className="mt-3 space-y-3">
            <div>
              <label className="block text-sm font-medium mb-2">
                OAuth Client ID
              </label>
              <Input
                value={formData.oauthClientId}
                onChange={(e) => onChange('oauthClientId', e.target.value)}
                placeholder="your-client-id"
                disabled={disabled}
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                OAuth Client Secret
              </label>
              <Input
                type="password"
                value={formData.oauthClientSecret}
                onChange={(e) => onChange('oauthClientSecret', e.target.value)}
                placeholder="your-client-secret"
                disabled={disabled}
                required
              />
              <p className="text-xs text-gray-500 mt-1">
                Secret will be stored securely in system keychain
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                Authorization URL
              </label>
              <Input
                value={formData.oauthAuthUrl}
                onChange={(e) => onChange('oauthAuthUrl', e.target.value)}
                placeholder="https://auth.example.com/authorize"
                disabled={disabled}
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                Token URL
              </label>
              <Input
                value={formData.oauthTokenUrl}
                onChange={(e) => onChange('oauthTokenUrl', e.target.value)}
                placeholder="https://auth.example.com/token"
                disabled={disabled}
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                Scopes (comma or newline separated)
              </label>
              <textarea
                value={formData.oauthScopes}
                onChange={(e) => onChange('oauthScopes', e.target.value)}
                placeholder="read&#10;write"
                className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
                rows={2}
                disabled={disabled}
              />
            </div>

            <div className="bg-yellow-900/20 border border-yellow-700 rounded p-3">
              <p className="text-yellow-200 text-sm">
                <strong>Note:</strong> This is for pre-registered OAuth credentials.
                OAuth flow authentication is not yet fully implemented.
              </p>
            </div>
          </div>
        )}

        {/* Environment Variables Auth (STDIO only) */}
        {formData.authMethod === 'env_vars' && (
          <div className="mt-3">
            <label className="block text-sm font-medium mb-2">
              Auth Environment Variables (KEY=VALUE, one per line)
            </label>
            <textarea
              value={formData.authEnvVars}
              onChange={(e) => onChange('authEnvVars', e.target.value)}
              placeholder="API_KEY=your-api-key&#10;AUTH_TOKEN=your-token"
              className="w-full px-3 py-2 bg-gray-800 border border-gray-700 rounded-md"
              rows={3}
              disabled={disabled}
              required
            />
            <p className="text-xs text-gray-500 mt-1">
              These will be merged with base environment variables
            </p>
          </div>
        )}
      </div>
    </div>
  )
}
