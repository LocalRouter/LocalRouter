/** Types for check-demo-mock-coverage.mjs, used by tests/e2e/unit/demo-mock-coverage.spec.ts. */

export interface DynamicCallSite {
  /** `file:line` of the call, relative to the repository root. */
  site: string
  /** Source text of the command-name argument. */
  text: string
  /** Wrapper function the call goes through, if any. */
  via?: string
}

export interface CoverageReport {
  registered: string[]
  mocked: string[]
  invoked: string[]
  callSites: Record<string, string[]>
  unmocked: string[]
  neverInvoked: string[]
  staleMocks: string[]
  invokedUnregistered: string[]
  mockedNeverInvoked: string[]
  staleAllowlist: string[]
  dynamic: DynamicCallSite[]
}

export const DELIBERATELY_UNMOCKED: Record<string, string>
export function registeredCommands(root?: string): Set<string>
export function mockedCommands(root?: string): Set<string>
export function invokedCommands(root?: string): { invoked: Map<string, string[]>; dynamic: DynamicCallSite[] }
export function analyze(root?: string): CoverageReport
