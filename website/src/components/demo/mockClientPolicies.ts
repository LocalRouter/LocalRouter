import type { NativeOperation, UpstreamToolsPolicy } from '@app/types/tauri-commands'

// Demo-only memory. These values never authorize a real LocalRouter server.
export const mockUpstreamTools = new Map<string, UpstreamToolsPolicy>()
export const mockClientPrivacy = new Map<string, { local_only: boolean; no_retention: boolean; metadata_only: boolean }>()
export const mockClientSecurity = new Map<string, { expiry: string | null; reviewDays: number | null; hasCredential: boolean }>()
export const mockClientSecrets = new Map<string, string>()

export const mockNativeOperations = new Map<string, NativeOperation[]>()
