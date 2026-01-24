import Input from '../ui/Input'
import Select from '../ui/Select'
import KeyValueInput from '../ui/KeyValueInput'

// Helper functions to convert between string format and Record<string, string>
function parseEnvVars(envVarsStr: string): Record<string, string> {
  const result: Record<string, string> = {}
  if (!envVarsStr.trim()) return result

  envVarsStr.split('\n').forEach(line => {
    const trimmed = line.trim()
    if (!trimmed) return
    const [key, ...valueParts] = trimmed.split('=')
    if (key && valueParts.length > 0) {
      result[key.trim()] = valueParts.join('=').trim()
    }
  })
  return result
}

function stringifyEnvVars(envVars: Record<string, string>): string {
  return Object.entries(envVars)
    .map(([k, v]) => `${k}=${v}`)
    .join('\n')
}

function parseHeaders(headersStr: string): Record<string, string> {
  const result: Record<string, string> = {}
  if (!headersStr.trim()) return result

  headersStr.split('\n').forEach(line => {
    const trimmed = line.trim()
    if (!trimmed) return
    const [key, ...valueParts] = trimmed.split(':')
    if (key && valueParts.length > 0) {
      result[key.trim()] = valueParts.join(':').trim()
    }
  })
  return result
}

function stringifyHeaders(headers: Record<string, string>): string {
  return Object.entries(headers)
    .map(([k, v]) => `${k}: ${v}`)
    .join('\n')
}

export interface McpConfigFormData {
  serverName: string
  transportType: 'Stdio' | 'Sse'
  // STDIO config
  command: string
  args: string
  envVars: string
  // SSE config
  url: string
  headers: string
  // Auth config
  authMethod: 'none' | 'bearer' | 'custom_headers' | 'oauth' | 'oauth_browser' | 'env_vars'
  bearerToken: string
  authHeaders: string
  authEnvVars: string
  oauthClientId: string
  oauthClientSecret: string
  oauthAuthUrl: string
  oauthTokenUrl: string
  oauthScopes: string
  // OAuth Browser fields
  oauthBrowserClientId: string
  oauthBrowserClientSecret: string
  oauthBrowserAuthUrl: string
  oauthBrowserTokenUrl: string
  oauthBrowserScopes: string
  oauthBrowserRedirectUri: string
}

interface McpConfigFormProps {
  formData: McpConfigFormData
  onChange: (field: keyof McpConfigFormData, value: string) => void
  disabled?: boolean
  showTransportType?: boolean
  disableTransportTypeChange?: boolean
}

export default function McpConfigForm({ formData, onChange, disabled = false, showTransportType = true, disableTransportTypeChange = false }: McpConfigFormProps) {
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
            disabled={disabled || disableTransportTypeChange}
          >
            <option value="Stdio">STDIO (Subprocess)</option>
            <option value="Sse">SSE (Server-Sent Events)</option>
          </Select>
          {disableTransportTypeChange && !disabled && (
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
              Changing transport type will require restarting the server
            </p>
          )}
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
              placeholder="npx -y @modelcontextprotocol/server-everything"
              disabled={disabled}
              required
            />
            <p className="text-xs text-muted-foreground mt-1">
              Full command with arguments (e.g., npx -y @modelcontextprotocol/server-filesystem /tmp)
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Environment Variables
            </label>
            <KeyValueInput
              value={parseEnvVars(formData.envVars)}
              onChange={(envVars) => onChange('envVars', stringifyEnvVars(envVars))}
              keyPlaceholder="KEY"
              valuePlaceholder="VALUE"
            />
          </div>
        </>
      )}

      {/* SSE Config */}
      {formData.transportType === 'Sse' && (
        <>
          <div>
            <label className="block text-sm font-medium mb-2">
              URL
            </label>
            <Input
              value={formData.url}
              onChange={(e) => onChange('url', e.target.value)}
              placeholder="http://localhost:3000"
              disabled={disabled}
              required
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Headers
            </label>
            <KeyValueInput
              value={parseHeaders(formData.headers)}
              onChange={(headers) => onChange('headers', stringifyHeaders(headers))}
              keyPlaceholder="Header Name"
              valuePlaceholder="Header Value"
            />
          </div>
        </>
      )}

      {/* Authentication Configuration */}
      <div className="border-t border-gray-700 dark:border-gray-600 pt-4 mt-4">
        <h3 className="text-md font-semibold mb-3">Authentication (Optional)</h3>
        <p className="text-sm text-gray-400 dark:text-gray-500 mb-3">
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
            <option value="none">None / Via headers</option>
            {formData.transportType !== 'Stdio' && <option value="bearer">Bearer Token</option>}
            {formData.transportType !== 'Stdio' && <option value="custom_headers">Custom Headers</option>}
            {formData.transportType !== 'Stdio' && <option value="oauth">OAuth (Client Credentials)</option>}
            {formData.transportType !== 'Stdio' && <option value="oauth_browser">OAuth (Browser Flow)</option>}
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
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
              Token will be stored securely in system keychain
            </p>
          </div>
        )}

        {/* Custom Headers Auth */}
        {formData.authMethod === 'custom_headers' && (
          <div className="mt-3">
            <label className="block text-sm font-medium mb-2">
              Auth Headers
            </label>
            <KeyValueInput
              value={parseHeaders(formData.authHeaders)}
              onChange={(headers) => onChange('authHeaders', stringifyHeaders(headers))}
              keyPlaceholder="Header Name"
              valuePlaceholder="Header Value"
            />
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
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
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Secret will be stored securely in system keychain
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                Scopes (comma or newline separated)
              </label>
              <textarea
                value={formData.oauthScopes}
                onChange={(e) => onChange('oauthScopes', e.target.value)}
                placeholder="read&#10;write"
                className="w-full px-3 py-2 bg-gray-800 dark:bg-gray-900 border border-gray-700 dark:border-gray-600 rounded-md text-gray-900 dark:text-gray-100"
                rows={2}
                disabled={disabled}
              />
            </div>

            <div className="bg-yellow-900/20 dark:bg-yellow-900/30 border border-yellow-700 dark:border-yellow-600 rounded p-3">
              <p className="text-yellow-200 dark:text-yellow-300 text-sm">
                <strong>Note:</strong> OAuth URLs are auto-discovered from the MCP server.
              </p>
            </div>
          </div>
        )}

        {/* OAuth Browser Auth */}
        {formData.authMethod === 'oauth_browser' && (
          <div className="mt-3 space-y-3">
            <div className="bg-blue-900/20 dark:bg-blue-900/30 border border-blue-700 dark:border-blue-600 rounded p-3">
              <p className="text-blue-200 dark:text-blue-300 text-sm">
                <strong>Browser-based OAuth:</strong> User will authenticate via browser when clicking "Authenticate" button.
                Endpoints will be auto-discovered from the server's <code className="text-xs bg-blue-800/30 px-1 rounded">.well-known/oauth-protected-resource</code> endpoint.
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                OAuth Client ID
              </label>
              <Input
                value={formData.oauthBrowserClientId}
                onChange={(e) => onChange('oauthBrowserClientId', e.target.value)}
                placeholder="your-oauth-app-client-id"
                disabled={disabled}
                required
              />
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Create an OAuth app in your provider (GitHub, GitLab, etc.)
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                OAuth Client Secret
              </label>
              <Input
                type="password"
                value={formData.oauthBrowserClientSecret}
                onChange={(e) => onChange('oauthBrowserClientSecret', e.target.value)}
                placeholder="your-oauth-app-client-secret"
                disabled={disabled}
                required
              />
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Secret will be stored securely in system keychain
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                Scopes (space-separated)
              </label>
              <Input
                value={formData.oauthBrowserScopes}
                onChange={(e) => onChange('oauthBrowserScopes', e.target.value)}
                placeholder="read write"
                disabled={disabled}
              />
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Optional: Leave blank to use default scopes
              </p>
            </div>

            <div>
              <label className="block text-sm font-medium mb-2">
                Redirect URI
              </label>
              <Input
                value={formData.oauthBrowserRedirectUri}
                onChange={(e) => onChange('oauthBrowserRedirectUri', e.target.value)}
                placeholder="http://localhost:8080/callback"
                disabled={disabled}
              />
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                Must match the redirect URI configured in your OAuth app (default: http://localhost:8080/callback)
              </p>
            </div>

            <div className="bg-gray-800/50 dark:bg-gray-900/50 border border-gray-700 dark:border-gray-600 rounded p-3">
              <p className="text-xs text-gray-400 dark:text-gray-500 mb-2">
                <strong>Setup Instructions:</strong>
              </p>
              <ol className="text-xs text-gray-400 dark:text-gray-500 space-y-1 list-decimal list-inside">
                <li>Create an OAuth application in your provider's developer settings</li>
                <li>Set the authorization callback URL to: <code className="bg-gray-700/50 px-1 rounded">http://localhost:8080/callback</code></li>
                <li>Copy the Client ID and Client Secret above</li>
                <li>Save the server configuration</li>
                <li>Click "Authenticate" button on the server detail page to complete authentication</li>
              </ol>
            </div>
          </div>
        )}

        {/* Environment Variables Auth (STDIO only) */}
        {formData.authMethod === 'env_vars' && (
          <div className="mt-3">
            <label className="block text-sm font-medium mb-2">
              Auth Environment Variables
            </label>
            <KeyValueInput
              value={parseEnvVars(formData.authEnvVars)}
              onChange={(envVars) => onChange('authEnvVars', stringifyEnvVars(envVars))}
              keyPlaceholder="KEY"
              valuePlaceholder="VALUE"
            />
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
              These will be merged with base environment variables
            </p>
          </div>
        )}
      </div>
    </div>
  )
}
