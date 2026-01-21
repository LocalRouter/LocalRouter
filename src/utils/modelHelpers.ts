/**
 * Model helper utilities for determining offline/local status
 * and other model-related operations.
 */

/**
 * Provider types that are always local (don't require internet)
 */
const ALWAYS_LOCAL_PROVIDERS = ['ollama', 'lmstudio'];

/**
 * Provider configuration structure from backend
 */
export interface ProviderConfig {
  instance_name: string;
  provider_type: string;
  config: Record<string, string>;
}

/**
 * Determines if a provider/model combination is offline/local.
 *
 * A model is considered offline/local if:
 * 1. The provider type is always local (ollama, lmstudio)
 * 2. The provider is openai_compatible with is_local=true flag
 * 3. The provider has a localhost-based base_url (fallback check)
 *
 * @param _provider - Provider name (e.g., "ollama", "openai", "my-local-server") - unused but kept for API clarity
 * @param providerType - Provider type (e.g., "ollama", "openai_compatible")
 * @param providerConfig - Provider configuration including base_url and is_local flag
 * @returns true if the model runs locally without internet
 */
export function isOfflineModel(
  _provider: string,
  providerType: string,
  providerConfig?: Record<string, string>
): boolean {
  // 1. Hard-coded local providers
  const normalizedType = providerType.toLowerCase();
  if (ALWAYS_LOCAL_PROVIDERS.includes(normalizedType)) {
    return true;
  }

  // 2. OpenAI-compatible providers with is_local flag
  if (normalizedType === 'openai_compatible' && providerConfig) {
    const isLocal = providerConfig.is_local;
    if (isLocal === 'true') {
      return true;
    }
  }

  // 3. Check if base_url is localhost (fallback for backward compat)
  if (providerConfig?.base_url) {
    const baseUrl = providerConfig.base_url.toLowerCase();
    if (
      baseUrl.includes('localhost') ||
      baseUrl.includes('127.0.0.1') ||
      baseUrl.includes('::1')
    ) {
      return true;
    }
  }

  // 4. All other providers are online-only
  return false;
}

/**
 * Checks if a prioritized models list contains at least one offline model.
 *
 * @param prioritizedModels - Array of [provider, model_id] tuples
 * @param providerConfigs - Map of provider names to their configurations
 * @returns true if at least one offline model is in the list
 */
export function hasOfflineModel(
  prioritizedModels: [string, string][],
  providerConfigs: Map<string, ProviderConfig>
): boolean {
  if (prioritizedModels.length === 0) {
    return false;
  }

  return prioritizedModels.some(([providerName, _modelId]) => {
    const config = providerConfigs.get(providerName);
    if (!config) {
      // If we can't find the provider config, assume it's online
      return false;
    }

    return isOfflineModel(providerName, config.provider_type, config.config);
  });
}

/**
 * Gets a list of all offline models from a prioritized models list.
 * Useful for showing which models are offline in the UI.
 *
 * @param prioritizedModels - Array of [provider, model_id] tuples
 * @param providerConfigs - Map of provider names to their configurations
 * @returns Array of offline model tuples
 */
export function getOfflineModels(
  prioritizedModels: [string, string][],
  providerConfigs: Map<string, ProviderConfig>
): [string, string][] {
  return prioritizedModels.filter(([providerName, _modelId]) => {
    const config = providerConfigs.get(providerName);
    if (!config) {
      return false;
    }

    return isOfflineModel(providerName, config.provider_type, config.config);
  });
}

/**
 * Gets a list of all online-only models from a prioritized models list.
 *
 * @param prioritizedModels - Array of [provider, model_id] tuples
 * @param providerConfigs - Map of provider names to their configurations
 * @returns Array of online-only model tuples
 */
export function getOnlineModels(
  prioritizedModels: [string, string][],
  providerConfigs: Map<string, ProviderConfig>
): [string, string][] {
  return prioritizedModels.filter(([providerName, _modelId]) => {
    const config = providerConfigs.get(providerName);
    if (!config) {
      // If we can't find the provider config, assume it's online
      return true;
    }

    return !isOfflineModel(providerName, config.provider_type, config.config);
  });
}
