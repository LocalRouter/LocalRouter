import { homedir } from 'os';
import { join } from 'path';
import fs from 'fs-extra';
import YAML from 'yaml';
import { Page } from '@playwright/test';

const TEST_CONFIG_DIR = join(homedir(), '.localrouter-test');
const CONFIG_FILE = join(TEST_CONFIG_DIR, 'settings.yaml');

export interface Client {
  client_id: string;
  name: string;
  description?: string;
  api_key?: string;
  enabled?: boolean;
}

export interface AppConfig {
  server?: {
    host?: string;
    port?: number;
  };
  providers?: unknown[];
  clients?: Client[];
  mcp_servers?: unknown[];
}

/**
 * Read and parse the test config file
 */
export async function getConfig(): Promise<AppConfig> {
  const content = await fs.readFile(CONFIG_FILE, 'utf-8');
  return YAML.parse(content) as AppConfig;
}

/**
 * Write config to the test config file
 */
export async function writeConfig(config: AppConfig): Promise<void> {
  const content = YAML.stringify(config);
  await fs.writeFile(CONFIG_FILE, content);
}

/**
 * Reset config to default empty state
 */
export async function resetConfig(): Promise<void> {
  const defaultConfig: AppConfig = {
    server: {
      host: '127.0.0.1',
      port: 3625,
    },
    providers: [],
    clients: [],
    mcp_servers: [],
  };
  await writeConfig(defaultConfig);
}

/**
 * Wait for a Tauri event to be emitted in the page context
 */
export async function waitForTauriEvent(
  page: Page,
  eventName: string,
  timeout: number = 5000
): Promise<unknown> {
  return page.evaluate(
    async ({ eventName, timeout }) => {
      return new Promise((resolve, reject) => {
        const timeoutId = setTimeout(() => {
          reject(new Error(`Timeout waiting for Tauri event: ${eventName}`));
        }, timeout);

        // @ts-expect-error - Tauri API is available in the page context
        window.__TAURI__.event.listen(eventName, (event: unknown) => {
          clearTimeout(timeoutId);
          resolve(event);
        });
      });
    },
    { eventName, timeout }
  );
}

/**
 * Get the test config directory path
 */
export function getTestConfigDir(): string {
  return TEST_CONFIG_DIR;
}

/**
 * Get the config file path
 */
export function getConfigFilePath(): string {
  return CONFIG_FILE;
}

/**
 * Wait for config file to be updated (for async writes)
 */
export async function waitForConfigUpdate(
  expectedCondition: (config: AppConfig) => boolean,
  timeout: number = 5000,
  interval: number = 100
): Promise<AppConfig> {
  const startTime = Date.now();

  while (Date.now() - startTime < timeout) {
    try {
      const config = await getConfig();
      if (expectedCondition(config)) {
        return config;
      }
    } catch {
      // Config file might be in the middle of a write
    }
    await new Promise(resolve => setTimeout(resolve, interval));
  }

  throw new Error(`Timeout waiting for config update after ${timeout}ms`);
}
