/**
 * Compression type definitions
 *
 * Tauri command types are imported from src/types/tauri-commands.ts
 * UI-specific types are defined here.
 */

// Re-export Tauri command types for convenience
// Rust source: crates/lr-compression/src/types.rs
export type {
  PromptCompressionConfig,
  CompressionStatus,
  CompressionTestResult,
} from '@/types/tauri-commands';

// Compression rate presets (shared across global and client views)
export const COMPRESSION_PRESETS = [
  { name: "Aggressive", value: 0.5 },
  { name: "Balanced", value: 0.75 },
  { name: "Light", value: 0.9 },
]

// Resource requirement constants per model (benchmarked on Apple Silicon, release build)
export const COMPRESSION_REQUIREMENTS = {
  bert: {
    DISK_GB: '~0.7',
    MEMORY_GB: '~0.7',
    COLD_START_SECS: '~1',
    PER_REQUEST_MS: '~10-100',
  },
  'xlm-roberta': {
    DISK_GB: '~2.2',
    MEMORY_GB: '~2',
    COLD_START_SECS: '~2.5',
    PER_REQUEST_MS: '~30-300',
  },
} as const;
