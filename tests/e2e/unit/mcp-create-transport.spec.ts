import { test, expect } from '@playwright/test'
import { resolveMcpCreateTransport } from '../../../src/utils/mcp-create-transport'
import type { McpServerTemplate } from '../../../src/components/mcp/McpServerTemplates'

const base: McpServerTemplate = {
  id: 'test', name: 'Test', description: '', category: 'development', icon: 'server',
  transport: 'Stdio', authMethod: 'none', command: 'server',
}

test('custom and marketplace commands keep environment and working directory', () => {
  expect(resolveMcpCreateTransport(null, {}, 'Stdio', 'server --flag', { API_KEY: 'secret' }, ' /project ', '')).toEqual({
    type: 'stdio', command: 'server --flag', env: { API_KEY: 'secret' }, cwd: '/project',
  })
  expect(resolveMcpCreateTransport(null, {}, 'Stdio', 'server', {}, ' ', '').cwd).toBeNull()
})

test('template tests use resolved arguments and environment rather than manual form values', () => {
  const template: McpServerTemplate = { ...base, args: ['--project', '{{project}}'], fields: [
    { id: 'project', type: 'arg', label: '', placeholder: '' },
    { id: 'API_KEY', type: 'env_var', label: '', placeholder: '' },
  ] }
  expect(resolveMcpCreateTransport(template, { project: 'demo', API_KEY: 'secret' }, 'Stdio', 'ignored', { OTHER: 'ignored' }, '', '')).toEqual({
    type: 'stdio', command: 'server --project demo', env: { API_KEY: 'secret' }, cwd: null,
  })
})

test('HTTP template tests use the same resolved query parameters as creation', () => {
  const template: McpServerTemplate = { ...base, transport: 'Sse', url: 'https://example.com/mcp', fields: [
    { id: 'tools', type: 'query', label: '', placeholder: '' },
  ] }
  expect(resolveMcpCreateTransport(template, { tools: 'search,read' }, 'Sse', '', {}, '', 'https://ignored.test')).toEqual({
    type: 'http_sse', url: 'https://example.com/mcp?tools=search,read', headers: {},
  })
})

test('custom HTTP URL is preserved without persisting test credentials in the transport', () => {
  expect(resolveMcpCreateTransport(null, {}, 'Sse', '', {}, '', 'https://example.com/mcp')).toEqual({
    type: 'http_sse', url: 'https://example.com/mcp', headers: {},
  })
})
