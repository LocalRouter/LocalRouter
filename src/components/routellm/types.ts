/**
 * RouteLLM type definitions
 *
 * Tauri command types are imported from src/types/tauri-commands.ts
 * UI-specific types are defined here.
 */

// Re-export Tauri command types for convenience
// Rust source: crates/lr-routellm/src/status.rs
export type {
  RouteLLMState,
  RouteLLMStatus,
  RouteLLMTestResult,
  RouteLLMTestPredictionParams,
  RouteLLMUpdateSettingsParams,
} from '@/types/tauri-commands';

// Resource requirement constants for RouteLLM
export const ROUTELLM_REQUIREMENTS = {
  DISK_GB: '~1',
  MEMORY_GB: '~1.3',
  COLD_START_SECS: '~1.5',
  PER_REQUEST_MS: '~10',
} as const;

// UI-specific types (not from Tauri commands)

export interface RouteLLMConfig {
  enabled: boolean;
  threshold: number;
  weak_models: [string, string][];
}

export interface ThresholdProfile {
  name: string;
  weak: number;
  strong: number;
  savings: string;
  quality: string;
}
