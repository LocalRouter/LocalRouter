import OpenAI from "openai";

export interface OpenAIClientConfig {
  apiKey: string;
  baseURL: string;
}

/**
 * Creates an OpenAI client configured for LocalRouter.
 * Uses dangerouslyAllowBrowser: true which is safe for Tauri desktop apps
 * since requests go to localhost and are user-controlled.
 */
export function createOpenAIClient(config: OpenAIClientConfig): OpenAI {
  return new OpenAI({
    apiKey: config.apiKey,
    baseURL: config.baseURL,
    dangerouslyAllowBrowser: true,
  });
}
