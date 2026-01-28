/**
 * MCP Server icon component - wrapper around ServiceIcon for backward compatibility
 */

import ServiceIcon from './ServiceIcon'

interface McpServerIconProps {
  serverName: string
  size?: number
  className?: string
}

export default function McpServerIcon({ serverName, size = 32, className = '' }: McpServerIconProps) {
  return <ServiceIcon service={serverName} size={size} className={className} fallbackToServerIcon />
}
