/** Apply remote-template query fields without losing existing parameters. */
export function resolveMcpTemplateUrl(
  template: { url?: string; fields?: { id: string; type: string }[] },
  values: Record<string, string>,
): string | undefined {
  if (!template.url) return undefined
  const url = new URL(template.url)
  for (const field of template.fields || []) {
    if (field.type !== 'query') continue
    const value = (values[field.id] || '').trim()
    if (value) url.searchParams.set(field.id, value)
    else url.searchParams.delete(field.id)
  }
  // Commas are safe in query values and keep toolset URLs readable.
  return url.toString().replace(/%2C/gi, ',')
}
