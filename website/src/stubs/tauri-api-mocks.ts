// Stub for @tauri-apps/api/mocks in demo mode
type MockHandler = (args: unknown) => unknown

const handlers: Record<string, MockHandler> = {}

export const mockIPC = (handler: (cmd: string, args: unknown) => unknown) => {
  // Store a generic handler that will be called for all commands
  (window as unknown as { __TAURI_IPC_HANDLER__: typeof handler }).__TAURI_IPC_HANDLER__ = handler
}

export const mockWindows = () => {}

export const clearMocks = () => {
  delete (window as unknown as { __TAURI_IPC_HANDLER__?: unknown }).__TAURI_IPC_HANDLER__
}
