import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'

interface LLMLogEntry {
  timestamp: string
  api_key_name: string
  provider: string
  model: string
  status: string
  status_code: number
  input_tokens: number
  output_tokens: number
  total_tokens: number
  cost_usd: number
  latency_ms: number
  request_id: string
  routellm_win_rate?: number
}

interface MCPLogEntry {
  timestamp: string
  client_id: string
  server_id: string
  method: string
  status: string
  status_code: number
  error_code?: number
  latency_ms: number
  transport: string
  request_id: string
}

interface FilteredAccessLogsProps {
  type: 'llm' | 'mcp'
  clientName?: string
  clientId?: string
  provider?: string
  model?: string
  serverId?: string
  active: boolean // Only load logs when this tab is active
}

export default function FilteredAccessLogs({
  type,
  clientName,
  clientId,
  provider,
  model,
  serverId,
  active,
}: FilteredAccessLogsProps) {
  const [llmLogs, setLlmLogs] = useState<LLMLogEntry[]>([])
  const [mcpLogs, setMcpLogs] = useState<MCPLogEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [limit, setLimit] = useState(100)

  useEffect(() => {
    // Only load logs when the tab is active
    if (active) {
      loadLogs()
    }
  }, [active, limit, clientName, clientId, provider, model, serverId])

  const loadLogs = async () => {
    setLoading(true)
    try {
      if (type === 'llm') {
        const logs = await invoke<LLMLogEntry[]>('get_llm_logs', {
          limit,
          offset: 0,
          clientName,
          provider,
          model,
        })
        setLlmLogs(logs)
      } else if (type === 'mcp') {
        const logs = await invoke<MCPLogEntry[]>('get_mcp_logs', {
          limit,
          offset: 0,
          clientId,
          serverId,
        })
        setMcpLogs(logs)
      }
    } catch (error) {
      console.error('Failed to load logs:', error)
    } finally {
      setLoading(false)
    }
  }

  const formatTimestamp = (timestamp: string) => {
    const date = new Date(timestamp)
    return date.toLocaleString()
  }

  const formatCost = (cost: number) => {
    return `$${cost.toFixed(6)}`
  }

  const formatLatency = (latencyMs: number) => {
    if (latencyMs < 1000) {
      return `${latencyMs}ms`
    }
    return `${(latencyMs / 1000).toFixed(2)}s`
  }

  if (!active) {
    return (
      <Card>
        <div className="text-center py-8 text-gray-500 dark:text-gray-400">
          Logs will load when you view this tab
        </div>
      </Card>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex justify-between items-center">
        <div>
          <h3 className="text-lg font-semibold text-gray-800 dark:text-gray-100">
            {type === 'llm' ? 'LLM Request' : 'MCP Request'} Logs
          </h3>
          <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">
            Filtered access logs for this resource
          </p>
        </div>
        <div className="flex gap-2 items-center">
          <select
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value))}
            className="px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg text-sm bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100"
          >
            <option value={50}>Last 50</option>
            <option value={100}>Last 100</option>
            <option value={500}>Last 500</option>
            <option value={1000}>Last 1000</option>
          </select>
          <Button onClick={loadLogs} disabled={loading}>
            {loading ? 'Refreshing...' : 'Refresh'}
          </Button>
        </div>
      </div>

      {type === 'llm' && (
        <Card>
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
              <thead className="bg-gray-50 dark:bg-gray-800">
                <tr>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Timestamp
                  </th>
                  {!clientName && (
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Client
                    </th>
                  )}
                  {!provider && (
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Provider
                    </th>
                  )}
                  {!model && (
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Model
                    </th>
                  )}
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Status
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Tokens
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Cost
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Latency
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    RouteLLM
                  </th>
                </tr>
              </thead>
              <tbody className="bg-white dark:bg-gray-900 divide-y divide-gray-200 dark:divide-gray-800">
                {llmLogs.length === 0 && !loading && (
                  <tr>
                    <td colSpan={9} className="px-6 py-12 text-center text-gray-500 dark:text-gray-400">
                      No logs found for this filter
                    </td>
                  </tr>
                )}
                {llmLogs.map((log, index) => (
                  <tr key={`${log.request_id}-${index}`} className="hover:bg-gray-50 dark:hover:bg-gray-800">
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                      {formatTimestamp(log.timestamp)}
                    </td>
                    {!clientName && (
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                        {log.api_key_name}
                      </td>
                    )}
                    {!provider && (
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                        {log.provider}
                      </td>
                    )}
                    {!model && (
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                        {log.model}
                      </td>
                    )}
                    <td className="px-6 py-4 whitespace-nowrap">
                      <Badge variant={log.status === 'success' ? 'success' : 'error'}>
                        {log.status}
                      </Badge>
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                      <div className="flex flex-col">
                        <span className="text-xs text-gray-500 dark:text-gray-400">In: {log.input_tokens}</span>
                        <span className="text-xs text-gray-500 dark:text-gray-400">Out: {log.output_tokens}</span>
                      </div>
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                      {formatCost(log.cost_usd)}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                      {formatLatency(log.latency_ms)}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm">
                      {log.routellm_win_rate !== undefined && log.routellm_win_rate !== null ? (
                        <Badge
                          variant={log.routellm_win_rate >= 0.5 ? 'warning' : 'success'}
                        >
                          {(log.routellm_win_rate * 100).toFixed(1)}%
                        </Badge>
                      ) : (
                        <span className="text-gray-400 dark:text-gray-500">-</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {type === 'mcp' && (
        <Card>
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
              <thead className="bg-gray-50 dark:bg-gray-800">
                <tr>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Timestamp
                  </th>
                  {!clientId && (
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Client
                    </th>
                  )}
                  {!serverId && (
                    <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                      Server
                    </th>
                  )}
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Method
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Transport
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Status
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Latency
                  </th>
                </tr>
              </thead>
              <tbody className="bg-white dark:bg-gray-900 divide-y divide-gray-200 dark:divide-gray-800">
                {mcpLogs.length === 0 && !loading && (
                  <tr>
                    <td colSpan={7} className="px-6 py-12 text-center text-gray-500 dark:text-gray-400">
                      No logs found for this filter
                    </td>
                  </tr>
                )}
                {mcpLogs.map((log, index) => (
                  <tr key={`${log.request_id}-${index}`} className="hover:bg-gray-50 dark:hover:bg-gray-800">
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                      {formatTimestamp(log.timestamp)}
                    </td>
                    {!clientId && (
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                        {log.client_id}
                      </td>
                    )}
                    {!serverId && (
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                        {log.server_id}
                      </td>
                    )}
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
                      {log.method}
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap">
                      <Badge variant="secondary">{log.transport}</Badge>
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap">
                      <Badge variant={log.status === 'success' ? 'success' : 'error'}>
                        {log.status}
                        {log.error_code && ` (${log.error_code})`}
                      </Badge>
                    </td>
                    <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
                      {formatLatency(log.latency_ms)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  )
}
