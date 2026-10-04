import { test, expect } from '@playwright/test'
import { resolveMcpTemplateUrl } from '../../../src/utils/mcp-template-url'

const datadog = {
  url: 'https://mcp.datadoghq.com/api/unstable/mcp-server/mcp',
  fields: [{ id: 'toolsets', type: 'query' }],
}

test('Datadog toolset selection keeps the endpoint and comma-separated values', () => {
  expect(resolveMcpTemplateUrl(datadog, { toolsets: 'ddsql' }))
    .toBe(`${datadog.url}?toolsets=ddsql`)
  expect(resolveMcpTemplateUrl(datadog, { toolsets: 'core,ddsql' }))
    .toBe(`${datadog.url}?toolsets=core,ddsql`)
  expect(resolveMcpTemplateUrl(datadog, { toolsets: 'all' }))
    .toBe(`${datadog.url}?toolsets=all`)
})

test('empty toolsets use provider defaults and preserve unrelated query parameters', () => {
  const template = { ...datadog, url: `${datadog.url}?subdomain=example&toolsets=ddsql` }
  expect(resolveMcpTemplateUrl(template, { toolsets: '' }))
    .toBe(`${datadog.url}?subdomain=example`)
})

test('remote URLs preserve version paths and safely encode field values', () => {
  expect(resolveMcpTemplateUrl({ url: 'https://mcp.atlassian.com/v2/mcp' }, {}))
    .toBe('https://mcp.atlassian.com/v2/mcp')
  const resolved = resolveMcpTemplateUrl(datadog, { toolsets: 'ddsql&subdomain=other' })!
  expect(new URL(resolved).searchParams.get('toolsets')).toBe('ddsql&subdomain=other')
  expect(new URL(resolved).searchParams.has('subdomain')).toBe(false)
  expect(resolveMcpTemplateUrl({ fields: [] }, {})).toBeUndefined()
})
