/**
 * RouteLLM type definitions
 */

// Resource requirement constants for RouteLLM
export const ROUTELLM_REQUIREMENTS = {
  DISK_GB: '~1',
  MEMORY_GB: '~1.3',
  COLD_START_SECS: '~1.5',
  PER_REQUEST_MS: '~10',
} as const;

export type RouteLLMState =
  | 'not_downloaded'
  | 'downloading'
  | 'downloaded_not_running'
  | 'initializing'
  | 'started';

export interface RouteLLMStatus {
  state: RouteLLMState;
  memory_usage_mb: number | null;
  last_access_secs_ago: number | null;
}

export interface RouteLLMTestResult {
  is_strong: boolean;
  win_rate: number;
  latency_ms: number;
}

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
