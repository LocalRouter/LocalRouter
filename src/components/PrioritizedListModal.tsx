import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface PrioritizedListModalProps {
  isOpen: boolean;
  onClose: () => void;
  apiKeyId: string;
  apiKeyName: string;
  onSuccess: () => void;
}

interface ModelRoutingConfig {
  active_strategy: 'available_models' | 'force_model' | 'prioritized_list';
  available_models: {
    all_provider_models: string[];
    individual_models: [string, string][];
  };
  forced_model: [string, string] | null;
  prioritized_models: [string, string][];
}

interface ModelInfo {
  provider: string;
  id: string;
  display_name: string;
}

export const PrioritizedListModal: React.FC<PrioritizedListModalProps> = ({
  isOpen,
  onClose,
  apiKeyId,
  apiKeyName,
  onSuccess,
}) => {
  const [prioritizedModels, setPrioritizedModels] = useState<[string, string][]>([]);
  const [availableModels, setAvailableModels] = useState<ModelInfo[]>([]);
  const [selectedAvailable, setSelectedAvailable] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load routing config and available models when modal opens
  useEffect(() => {
    if (isOpen) {
      loadData();
    }
  }, [isOpen, apiKeyId]);

  const loadData = async () => {
    try {
      setError(null);

      // Get routing config for this API key
      const config = await invoke<ModelRoutingConfig | null>('get_routing_config', {
        id: apiKeyId,
      });

      if (config) {
        setPrioritizedModels(config.prioritized_models || []);
      }

      // Get all available models
      const models = await invoke<ModelInfo[]>('list_all_models');
      setAvailableModels(models);
    } catch (err) {
      console.error('Failed to load data:', err);
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleMoveUp = (index: number) => {
    if (index === 0) return;
    const newList = [...prioritizedModels];
    [newList[index - 1], newList[index]] = [newList[index], newList[index - 1]];
    setPrioritizedModels(newList);
  };

  const handleMoveDown = (index: number) => {
    if (index === prioritizedModels.length - 1) return;
    const newList = [...prioritizedModels];
    [newList[index], newList[index + 1]] = [newList[index + 1], newList[index]];
    setPrioritizedModels(newList);
  };

  const handleRemove = (index: number) => {
    const newList = prioritizedModels.filter((_, i) => i !== index);
    setPrioritizedModels(newList);
  };

  const handleAdd = () => {
    if (!selectedAvailable) return;

    const [provider, model] = selectedAvailable.split('/');
    if (!provider || !model) return;

    // Check if already in list
    const alreadyExists = prioritizedModels.some(
      ([p, m]) => p === provider && m === model
    );

    if (alreadyExists) {
      setError('Model is already in the prioritized list');
      return;
    }

    setPrioritizedModels([...prioritizedModels, [provider, model]]);
    setSelectedAvailable('');
    setError(null);
  };

  const handleSave = async () => {
    try {
      setIsSaving(true);
      setError(null);

      // Update prioritized list
      await invoke('update_prioritized_list', {
        id: apiKeyId,
        prioritizedModels,
      });

      // Set strategy to prioritized_list
      await invoke('set_routing_strategy', {
        id: apiKeyId,
        strategy: 'prioritized_list',
      });

      onSuccess();
      onClose();
    } catch (err) {
      console.error('Failed to save prioritized list:', err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsSaving(false);
    }
  };

  if (!isOpen) return null;

  // Filter out models that are already in the prioritized list
  const filteredAvailable = availableModels.filter(
    (model) =>
      !prioritizedModels.some(([p, m]) => p === model.provider && m === model.id)
  );

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg shadow-xl w-full max-w-2xl max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="px-6 py-4 border-b border-gray-200">
          <h2 className="text-xl font-semibold text-gray-900">
            Prioritized List - {apiKeyName}
          </h2>
          <p className="text-sm text-gray-500 mt-1">
            Models are tried in order from top to bottom. If one fails, the next is tried
            automatically.
          </p>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {error && (
            <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded-md">
              <p className="text-sm text-red-700">{error}</p>
            </div>
          )}

          {/* Prioritized Models List */}
          <div className="mb-6">
            <h3 className="text-sm font-medium text-gray-700 mb-2">
              Prioritized Models ({prioritizedModels.length})
            </h3>
            {prioritizedModels.length === 0 ? (
              <div className="text-center py-8 text-gray-500 border border-dashed border-gray-300 rounded-md">
                <p>No models in the prioritized list</p>
                <p className="text-sm mt-1">Add models from the list below</p>
              </div>
            ) : (
              <div className="space-y-2">
                {prioritizedModels.map(([provider, model], index) => (
                  <div
                    key={`${provider}-${model}-${index}`}
                    className="flex items-center gap-2 p-3 bg-gray-50 border border-gray-200 rounded-md"
                  >
                    <span className="text-sm font-mono text-gray-500 w-6">
                      {index + 1}.
                    </span>
                    <span className="flex-1 text-sm">
                      {model} <span className="text-gray-500">({provider})</span>
                    </span>
                    <div className="flex gap-1">
                      <button
                        onClick={() => handleMoveUp(index)}
                        disabled={index === 0}
                        className="px-2 py-1 text-xs text-gray-600 hover:bg-gray-200 rounded disabled:opacity-30 disabled:cursor-not-allowed"
                        title="Move up"
                      >
                        ↑
                      </button>
                      <button
                        onClick={() => handleMoveDown(index)}
                        disabled={index === prioritizedModels.length - 1}
                        className="px-2 py-1 text-xs text-gray-600 hover:bg-gray-200 rounded disabled:opacity-30 disabled:cursor-not-allowed"
                        title="Move down"
                      >
                        ↓
                      </button>
                      <button
                        onClick={() => handleRemove(index)}
                        className="px-2 py-1 text-xs text-red-600 hover:bg-red-100 rounded"
                        title="Remove"
                      >
                        ✕
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Add Model Section */}
          <div>
            <h3 className="text-sm font-medium text-gray-700 mb-2">Add Model</h3>
            <div className="flex gap-2">
              <select
                value={selectedAvailable}
                onChange={(e) => setSelectedAvailable(e.target.value)}
                className="flex-1 px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="">Select a model to add...</option>
                {filteredAvailable.map((model) => (
                  <option
                    key={`${model.provider}/${model.id}`}
                    value={`${model.provider}/${model.id}`}
                  >
                    {model.id} ({model.provider})
                  </option>
                ))}
              </select>
              <button
                onClick={handleAdd}
                disabled={!selectedAvailable}
                className="px-4 py-2 bg-blue-600 text-white text-sm font-medium rounded-md hover:bg-blue-700 disabled:bg-gray-300 disabled:cursor-not-allowed"
              >
                Add
              </button>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="px-6 py-4 border-t border-gray-200 flex justify-end gap-3">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50"
            disabled={isSaving}
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={isSaving || prioritizedModels.length === 0}
            className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:bg-gray-300 disabled:cursor-not-allowed"
          >
            {isSaving ? 'Saving...' : 'Save & Enable'}
          </button>
        </div>
      </div>
    </div>
  );
};
