// Window operations share the demo's IPC and event mocks so desktop views can
// exercise their lifecycle in browser regression tests.
import { invoke } from './tauri-api-core'
import { listen } from './tauri-api-event'
import { currentWindowLabel } from './tauri-api-mocks'

export const getCurrentWebviewWindow = () => ({
  label: currentWindowLabel,
  close: () => invoke('plugin:window|close'),
  destroy: () => invoke('plugin:window|destroy'),
  hide: () => invoke('plugin:window|hide'),
  show: () => invoke('plugin:window|show'),
  setFocus: () => invoke('plugin:window|set_focus'),
  setSize: (size: unknown) => invoke('plugin:window|set_size', { size }),
  center: () => invoke('plugin:window|center'),
  isFocused: () => invoke<boolean>('plugin:window|is_focused'),
  listen,
  onFocusChanged: (callback: (event: { payload: boolean }) => void) => listen('tauri://focus', callback),
  onCloseRequested: (callback: (event: { preventDefault: () => void }) => void) =>
    listen('tauri://close-requested', () => callback({ preventDefault: () => {} })),
})
