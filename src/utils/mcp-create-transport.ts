import type { McpServerTemplate } from '@/components/mcp/McpServerTemplates'
import { resolveMcpTemplateUrl } from './mcp-template-url'

export function resolveMcpCreateTransport(
  template: McpServerTemplate | null | undefined,
  templateFieldValues: Record<string, string>,
  transportType: 'Stdio' | 'Sse',
  command: string,
  envVars: Record<string, string>,
  cwd: string,
  url: string,
) {
  let transportConfig
  if (template?.fields?.length) {
    // Build transport config from template + field values
    if (template.transport === "Stdio") {
      let resolvedArgs = [...(template.args || [])]
      const envVarsFromFields: Record<string, string> = {}

      template.fields.forEach(field => {
        const value = templateFieldValues[field.id] || ""
        if (field.type === "arg") {
          resolvedArgs = resolvedArgs.map(arg =>
            arg.includes(`{{${field.id}}}`) ? arg.replace(`{{${field.id}}}`, value) : arg
          )
        } else if (field.type === "env_var" && value) {
          envVarsFromFields[field.id] = value
        }
      })

      const fullCommand = [template.command, ...resolvedArgs].join(" ")
      transportConfig = { type: "stdio" as const, command: fullCommand, env: envVarsFromFields, cwd: cwd.trim() || null }
    } else {
      transportConfig = { type: "http_sse" as const, url: resolveMcpTemplateUrl(template, templateFieldValues) || url, headers: {} }
    }
  } else if (transportType === "Stdio") {
    transportConfig = { type: "stdio" as const, command, env: envVars, cwd: cwd.trim() || null }
  } else {
    transportConfig = { type: "http_sse" as const, url, headers: {} }
  }
  return transportConfig
}
