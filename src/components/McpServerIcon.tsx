/**
 * MCP Server icon component - wrapper around ServiceIcon for backward compatibility
 */

import ServiceIcon from './ServiceIcon'

interface McpServerIconProps {
  serverName: string
  serverId?: string
  size?: number
  className?: string
}

export default function McpServerIcon({ serverName, serverId, size = 32, className = '' }: McpServerIconProps) {
  return <ServiceIcon resourceKey={serverId ? `mcp:${serverId}` : undefined} service={serverName} size={size} className={className} fallbackToServerIcon />
}
