// Stub for @tauri-apps/api/core in demo mode
// Integrates with mockIPC from @tauri-apps/api/mocks

type IpcHandler = (cmd: string, args: unknown) => unknown

export const invoke = async <T>(cmd: string, args?: unknown): Promise<T> => {
  const handler = (window as unknown as { __TAURI_IPC_HANDLER__?: IpcHandler }).__TAURI_IPC_HANDLER__
  if (handler) {
    return handler(cmd, args) as T
  }
  return null as T
}

export type InvokeArgs = Record<string, unknown>
