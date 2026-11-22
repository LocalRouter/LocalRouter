import { test, expect } from '@playwright/test'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { analyze, DELIBERATELY_UNMOCKED } from '../../../scripts/check-demo-mock-coverage.mjs'

// The website demo runs the real frontend against `mockHandlers`
// (website/src/components/demo/TauriMockSetup.ts). A command the frontend
// invokes without a handler there shows a "Demo: not implemented" toast.
// Run `node scripts/check-demo-mock-coverage.mjs --where` for the full report.

test.describe('demo mock coverage', () => {
  const report = analyze()

  test('reads all three command sets', () => {
    // Guards against the parser silently finding nothing.
    expect(report.registered.length).toBeGreaterThan(400)
    expect(report.invoked.length).toBeGreaterThan(300)
    expect(report.mocked.length).toBeGreaterThan(300)
    for (const command of ['list_clients', 'get_config', 'get_server_status']) {
      expect(report.registered, command).toContain(command)
      expect(report.invoked, command).toContain(command)
      expect(report.mocked, command).toContain(command)
    }
    // Mocks contributed by a spread sibling module (mockReplay).
    expect(report.mocked).toContain('replay_list')
    // Names that never appear inside an `invoke(...)` call: chosen in a
    // helper's returned object (metrics-chart) or passed as a hook option
    // (useModelDownload).
    expect(report.invoked).toContain('get_global_metrics')
    expect(report.invoked).toContain('get_mcp_method_breakdown')
    expect(report.invoked).toContain('install_embedding_model')
    expect(report.mocked.some(name => name.startsWith('plugin:'))).toBe(false)
  })

  test('every command the frontend invokes has a demo mock', () => {
    expect(
      report.unmocked,
      'Add a typed handler to mockHandlers in website/src/components/demo/TauriMockSetup.ts ' +
        '(see AGENTS.md "Adding/Modifying Tauri Commands"), or list the command in ' +
        'DELIBERATELY_UNMOCKED in scripts/check-demo-mock-coverage.mjs with a reason.',
    ).toEqual([])
  })

  test('every invoke call site names its command statically', () => {
    // An unresolved name would hide a command from the check above. Teach
    // scripts/check-demo-mock-coverage.mjs the new pattern, or pass a literal.
    expect(report.dynamic).toEqual([])
  })

  test('no mock outlives its command', () => {
    expect(report.staleMocks, 'These mocks answer commands that are not in generate_handler!; remove them.').toEqual([])
  })

  test('the frontend invokes only registered commands', () => {
    expect(report.invokedUnregistered, 'These invoke() calls always fail in the app: the command is not registered.').toEqual([])
  })

  test('the allowlist only holds invoked, unmocked commands with a reason', () => {
    expect(report.staleAllowlist).toEqual([])
    for (const [command, reason] of Object.entries(DELIBERATELY_UNMOCKED)) {
      expect(reason.trim().length, `${command} needs a reason`).toBeGreaterThan(10)
    }
  })
})

test.describe('demo mock coverage analysis', () => {
  let root: string

  const write = (file: string, content: string) => {
    const full = path.join(root, file)
    mkdirSync(path.dirname(full), { recursive: true })
    writeFileSync(full, content)
  }

  test.beforeAll(() => {
    root = mkdtempSync(path.join(tmpdir(), 'demo-mock-coverage-'))
    write('src-tauri/src/main.rs', `
      fn main() {
          tauri::Builder::default()
              .invoke_handler(tauri::generate_handler![
                  ui::commands::plain,
                  // ui::commands::commented_out,
                  ui::commands::generic,
                  /* ui::commands::block_commented, */
                  ui::commands::multiline,
                  #[cfg(debug_assertions)]
                  ui::commands_debug::aliased,
                  ui::commands::ternary_a,
                  ui::commands::ternary_b,
                  ui::commands::from_const,
                  ui::commands::from_let_a,
                  ui::commands::from_let_b,
                  ui::commands::returned_a,
                  ui::commands::returned_b,
                  ui::commands::option_a,
                  ui::commands::positional_a,
                  ui::commands::imported_const,
                  ui::commands::website_only,
                  ui::commands::unmocked,
                  ui::commands::never_invoked,
                  ui::commands::never_invoked_but_mocked,
                  ui::commands::in_test_file_only,
              ])
              .run(tauri::generate_context!());
      }
    `)
    write('website/src/components/demo/TauriMockSetup.ts', `
      import { mockSibling } from './mockSibling'
      const mockHandlers: Record<string, (args?: any) => unknown> = {
        ...mockSibling,
        'plain': () => null,
        generic: () => ({ nested: { 'not_a_command': 1 } }),
        'multiline': () => null,
        'aliased': () => null,
        'ternary_a': () => null,
        'ternary_b': () => null,
        'from_const': () => null,
        'from_let_a': () => null,
        'from_let_b': () => null,
        'returned_a': () => null,
        'returned_b': () => null,
        'option_a': () => null,
        'positional_a': () => null,
        'imported_const': () => null,
        'never_invoked_but_mocked': () => null,
        'stale_mock': () => null,
        'plugin:event|listen': () => 0,
      }
      mockHandlers['website_only'] = () => null
      export { mockHandlers }
    `)
    write('website/src/components/demo/mockSibling.ts', `
      export const mockSibling = { 'from_sibling_stale': () => null }
    `)
    write('src/constants.ts', `export const IMPORTED = 'imported_const'`)
    write('src/wrappers.ts', `
      import { invoke } from '@tauri-apps/api/core'
      export function useDownload(config: { command: string; label: string }) {
        const { command } = config
        return () => invoke(command)
      }
      export const call = <T,>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args)
    `)
    write('src/app.tsx', `
      import { invoke as tauriInvoke } from '@tauri-apps/api/core'
      import { IMPORTED } from './constants'
      import { call, useDownload } from './wrappers'

      const FROM_CONST = 'from_const'

      export async function run(flag: boolean, unknown: string) {
        await tauriInvoke('plain')
        // await tauriInvoke('commented_call')
        const text = "invoke('inside_a_string')"
        await tauriInvoke<Record<string, Array<number>>>('generic', { a: 1 })
        await tauriInvoke<string>(
          'multiline',
          { b: 2 },
        )
        await tauriInvoke('aliased')
        await tauriInvoke(flag ? 'ternary_a' : 'ternary_b')
        await tauriInvoke(FROM_CONST)
        await tauriInvoke(IMPORTED)
        let cmd = 'from_let_a'
        if (flag) cmd = 'from_let_b'
        await tauriInvoke(cmd)
        const pick = () => {
          let command = ''
          if (flag) command = 'returned_a'
          else command = 'returned_b'
          return { command, args: {} }
        }
        const { command, args } = pick()
        await tauriInvoke(command, args)
        useDownload({ command: 'option_a', label: text })
        await call<number>('positional_a')
        await tauriInvoke('unmocked')
        await tauriInvoke('unregistered')
        await tauriInvoke(unknown)
        await tauriInvoke('plugin:window|close')
      }

      // Not Tauri's invoke: a local function with the same name is ignored.
      export function other(invoke: (name: string) => void) { invoke('not_tauri') }
    `)
    write('src/app.test.ts', `
      import { invoke } from '@tauri-apps/api/core'
      void invoke('in_test_file_only')
    `)
    write('website/src/page.tsx', `
      import { invoke } from '@tauri-apps/api/core'
      export const load = () => invoke('website_only')
    `)
  })

  test.afterAll(() => {
    rmSync(root, { recursive: true, force: true })
  })

  test('extracts registered commands, skipping comments and attributes', () => {
    const { registered } = analyze(root)
    expect(registered).toContain('plain')
    expect(registered).toContain('aliased')
    expect(registered).not.toContain('commented_out')
    expect(registered).not.toContain('block_commented')
    expect(registered).toHaveLength(19)
  })

  test('extracts mock keys from the dispatcher, its spreads and later assignments', () => {
    const { mocked } = analyze(root)
    expect(mocked).toContain('plain')
    expect(mocked).toContain('generic')
    expect(mocked).toContain('from_sibling_stale')
    expect(mocked).toContain('website_only')
    expect(mocked).not.toContain('not_a_command')
    expect(mocked).not.toContain('nested')
    expect(mocked).not.toContain('plugin:event|listen')
  })

  test('resolves every supported way of naming an invoked command', () => {
    const { invoked, callSites } = analyze(root)
    expect(invoked).toEqual([
      'aliased', 'from_const', 'from_let_a', 'from_let_b', 'generic', 'imported_const', 'multiline',
      'option_a', 'plain', 'positional_a', 'returned_a', 'returned_b', 'ternary_a', 'ternary_b',
      'unmocked', 'unregistered', 'website_only',
    ])
    expect(callSites.website_only).toEqual(['website/src/page.tsx:3'])
    // Wrapper-carried names are attributed to the wrapper's caller.
    expect(callSites.option_a[0]).toMatch(/^src\/app\.tsx:\d+$/)
    expect(callSites.positional_a[0]).toMatch(/^src\/app\.tsx:\d+$/)
  })

  test('reports each kind of gap', () => {
    const result = analyze(root)
    expect(result.unmocked).toEqual(['unmocked', 'unregistered'])
    expect(result.staleMocks).toEqual(['from_sibling_stale', 'stale_mock'])
    expect(result.invokedUnregistered).toEqual(['unregistered'])
    expect(result.neverInvoked).toEqual(['in_test_file_only', 'never_invoked', 'never_invoked_but_mocked'])
    expect(result.mockedNeverInvoked).toEqual(['never_invoked_but_mocked'])
    expect(result.dynamic).toHaveLength(1)
    expect(result.dynamic[0].text).toBe('unknown')
    expect(result.dynamic[0].via).toBe('run')
    expect(result.dynamic[0].site).toMatch(/^src\/app\.tsx:\d+$/)
  })
})
