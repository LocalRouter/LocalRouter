import type { ServerConfigInfo } from '@/types/tauri-commands'

export function gatewayBaseUrl(config: ServerConfigInfo): string {
  const host = config.host === '0.0.0.0' ? '127.0.0.1' : config.host === '::' ? '[::1]' : config.host.includes(':') && !config.host.startsWith('[') ? `[${config.host}]` : config.host
  return `http://${host}:${config.actual_port ?? config.port}`
}
export function gatewayPortExplanation(config: ServerConfigInfo): string | null {
  if (config.port !== 0 || config.actual_port == null) return null
  return `The operating system assigned port ${config.actual_port}.`
}
