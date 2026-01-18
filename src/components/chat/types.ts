/**
 * Chat component type definitions
 */

export interface ApiKeyContext {
  type: 'api_key';
  apiKeyId: string;
  apiKeyName: string;
  modelSelection: any; // API key's model selection config
  bearerToken?: string; // Optional: directly provide the bearer token instead of fetching from keychain
}

export interface ProviderContext {
  type: 'provider';
  instanceName: string;
  providerType: string;
  models: Array<{ model_id: string; provider_instance: string }>;
}

export interface ModelContext {
  type: 'model';
  providerInstance: string;
  modelId: string;
}

export type ChatContext = ApiKeyContext | ProviderContext | ModelContext;

export interface ContextualChatProps {
  context: ChatContext;
  disabled?: boolean;
}

export interface ModelSelectorProps {
  models: Array<{ model_id: string; provider_instance: string }>;
  selectedModel: string | null;
  onModelChange: (modelId: string) => void;
  disabled?: boolean;
  label?: string;
}

export interface ModelUsedDisplayProps {
  requestedModel: string;
  actualModel: string | null;
  contextType: ChatContext['type'];
}
