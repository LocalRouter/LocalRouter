/**
 * RouteLLM Configuration Editor Component
 * Main component for configuring RouteLLM settings
 */

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-shell';
import { RouteLLMStatusIndicator } from './RouteLLMStatusIndicator';
import { RouteLLMValueProp } from './RouteLLMValueProp';
import { ThresholdSlider } from './ThresholdSlider';
import { RouteLLMTester } from './RouteLLMTester';
import { RouteLLMConfig, RouteLLMStatus } from './types';

interface RouteLLMConfigEditorProps {
  config: RouteLLMConfig;
  onChange: (config: RouteLLMConfig) => void;
  showGlobalSettings?: boolean;
  availableModels?: Array<[string, string]>;
}

export const RouteLLMConfigEditor: React.FC<RouteLLMConfigEditorProps> = ({
  config,
  onChange,
  showGlobalSettings = false,
  availableModels = [],
}) => {
  const [status, setStatus] = useState<RouteLLMStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [modelPath, setModelPath] = useState<string>('');

  useEffect(() => {
    loadStatus();
    // Get model path from config (default location)
    const homeDir = '~'; // Will be replaced with actual home dir
    setModelPath(`${homeDir}/.localrouter/routellm/`);
  }, []);

  const loadStatus = async () => {
    try {
      const routellmStatus = await invoke<RouteLLMStatus>('routellm_get_status');
      setStatus(routellmStatus);
    } catch (error) {
      console.error('Failed to load RouteLLM status:', error);
    }
  };

  const handleDownload = async () => {
    setLoading(true);
    try {
      await invoke('routellm_download_models');
      await loadStatus();
    } catch (error) {
      alert(`Download failed: ${error}`);
    } finally {
      setLoading(false);
    }
  };

  const handleUnload = async () => {
    try {
      await invoke('routellm_unload');
      await loadStatus();
    } catch (error) {
      alert(`Unload failed: ${error}`);
    }
  };

  const handleOpenFolder = async () => {
    try {
      // Open the model folder in file explorer
      const homeDir = await invoke<string>('get_home_dir');
      const path = modelPath.replace('~', homeDir);
      await open(path);
    } catch (error) {
      console.error('Failed to open folder:', error);
    }
  };

  const addModel = (list: 'strong' | 'weak', provider: string, model: string) => {
    const models = list === 'strong' ? config.strong_models : config.weak_models;
    if (!models.some(([p, m]) => p === provider && m === model)) {
      onChange({
        ...config,
        [list === 'strong' ? 'strong_models' : 'weak_models']: [
          ...models,
          [provider, model] as [string, string],
        ],
      });
    }
  };

  const removeModel = (list: 'strong' | 'weak', index: number) => {
    const models = list === 'strong' ? config.strong_models : config.weak_models;
    onChange({
      ...config,
      [list === 'strong' ? 'strong_models' : 'weak_models']: models.filter((_, i) => i !== index),
    });
  };

  return (
    <div className="space-y-6">
      {/* Status Indicator */}
      {status && (
        <RouteLLMStatusIndicator
          status={status}
          modelPath={modelPath}
          onOpenFolder={handleOpenFolder}
        />
      )}

      {/* Value Proposition */}
      {!config.enabled && <RouteLLMValueProp />}

      {/* Enable Toggle */}
      <div className="flex items-center justify-between p-4 bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700">
        <div>
          <label className="text-sm font-medium text-gray-900 dark:text-gray-100">
            Enable RouteLLM Intelligent Routing
          </label>
          <p className="text-xs text-gray-600 dark:text-gray-400 mt-1">
            Use ML-based routing to optimize costs while maintaining quality
          </p>
        </div>
        <button
          onClick={() => onChange({ ...config, enabled: !config.enabled })}
          className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
            config.enabled ? 'bg-blue-600' : 'bg-gray-200 dark:bg-gray-700'
          }`}
        >
          <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white dark:bg-gray-200 transition-transform ${
              config.enabled ? 'translate-x-6' : 'translate-x-1'
            }`}
          />
        </button>
      </div>

      {config.enabled && (
        <>
          {/* Download Models */}
          {status?.state === 'not_downloaded' && (
            <div className="p-4 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-700 rounded-lg">
              <h4 className="font-semibold text-yellow-900 dark:text-yellow-100 mb-2">
                Models Required
              </h4>
              <p className="text-sm text-yellow-800 dark:text-yellow-200 mb-3">
                RouteLLM requires downloading model files (~1.08 GB) before it can be used.
              </p>
              <button
                onClick={handleDownload}
                disabled={loading}
                className="px-4 py-2 bg-yellow-600 text-white rounded-lg hover:bg-yellow-700 dark:bg-yellow-600 dark:hover:bg-yellow-700 disabled:bg-gray-400 dark:disabled:bg-gray-600 disabled:cursor-not-allowed transition-colors font-medium"
              >
                {loading ? 'Downloading...' : 'Download Models'}
              </button>
            </div>
          )}

          {/* Resource Requirements Warning */}
          <div className="p-4 bg-orange-50 dark:bg-orange-900/20 border border-orange-200 dark:border-orange-700 rounded-lg">
            <h4 className="font-semibold text-orange-800 dark:text-orange-100 mb-2">
              Resource Requirements
            </h4>
            <div className="grid grid-cols-2 gap-2 text-xs text-orange-700 dark:text-orange-200">
              <div>
                <strong>Cold Start:</strong> ~1.5s
              </div>
              <div>
                <strong>Disk Space:</strong> 1.08 GB
              </div>
              <div>
                <strong>Latency:</strong> ~10ms per request
              </div>
              <div>
                <strong>Memory:</strong> ~2.65 GB (when loaded)
              </div>
            </div>
            {status?.state === 'started' && (
              <button
                onClick={handleUnload}
                className="mt-3 text-xs px-3 py-1 bg-orange-600 text-white rounded hover:bg-orange-700 dark:bg-orange-600 dark:hover:bg-orange-700 transition-colors"
              >
                Unload Models (Free Memory)
              </button>
            )}
          </div>

          {/* Threshold Slider */}
          <ThresholdSlider value={config.threshold} onChange={(threshold) => onChange({ ...config, threshold })} />

          {/* Model Selection */}
          <div className="space-y-4">
            <div>
              <h4 className="font-semibold text-gray-900 dark:text-gray-100 mb-2">
                Strong Models (High Quality)
              </h4>
              <p className="text-xs text-gray-600 dark:text-gray-400 mb-2">
                Used when prompts are complex or require high-quality output
              </p>
              <ModelList
                models={config.strong_models}
                availableModels={availableModels}
                onAdd={(provider, model) => addModel('strong', provider, model)}
                onRemove={(index) => removeModel('strong', index)}
              />
            </div>

            <div>
              <h4 className="font-semibold text-gray-900 dark:text-gray-100 mb-2">
                Weak Models (Cost Efficient)
              </h4>
              <p className="text-xs text-gray-600 dark:text-gray-400 mb-2">
                Used when prompts are simple and straightforward
              </p>
              <ModelList
                models={config.weak_models}
                availableModels={availableModels}
                onAdd={(provider, model) => addModel('weak', provider, model)}
                onRemove={(index) => removeModel('weak', index)}
              />
            </div>
          </div>

          {/* Try It Out */}
          {status?.state === 'started' || status?.state === 'downloaded_not_running' ? (
            <RouteLLMTester threshold={config.threshold} />
          ) : null}
        </>
      )}

      {/* Global Settings */}
      {showGlobalSettings && (
        <div className="pt-6 border-t border-gray-200 dark:border-gray-700 space-y-4">
          <h4 className="font-semibold text-gray-900 dark:text-gray-100">Global Settings</h4>

          <div>
            <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
              Auto-Unload After Idle
            </label>
            <p className="text-xs text-gray-600 dark:text-gray-400 mt-1 mb-2">
              Automatically unload models from memory after inactivity
            </p>
            <select
              className="w-full p-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100"
              defaultValue="600"
            >
              <option value="300">5 minutes</option>
              <option value="600">10 minutes (recommended)</option>
              <option value="1800">30 minutes</option>
              <option value="3600">1 hour</option>
              <option value="0">Never</option>
            </select>
          </div>
        </div>
      )}
    </div>
  );
};

// Helper component for model lists
interface ModelListProps {
  models: Array<[string, string]>;
  availableModels: Array<[string, string]>;
  onAdd: (provider: string, model: string) => void;
  onRemove: (index: number) => void;
}

const ModelList: React.FC<ModelListProps> = ({ models, availableModels, onAdd, onRemove }) => {
  const [showAdd, setShowAdd] = useState(false);
  const [selectedModel, setSelectedModel] = useState<[string, string] | null>(null);

  return (
    <div className="space-y-2">
      {models.length === 0 ? (
        <div className="p-3 text-sm text-gray-500 dark:text-gray-400 bg-gray-50 dark:bg-gray-800 rounded border border-gray-200 dark:border-gray-700 text-center">
          No models configured
        </div>
      ) : (
        <div className="space-y-1">
          {models.map(([provider, model], index) => (
            <div
              key={index}
              className="flex items-center justify-between p-2 bg-gray-50 dark:bg-gray-800 rounded border border-gray-200 dark:border-gray-700"
            >
              <span className="text-sm font-mono text-gray-900 dark:text-gray-100">
                {provider}/{model}
              </span>
              <button
                onClick={() => onRemove(index)}
                className="text-red-600 dark:text-red-400 hover:text-red-700 dark:hover:text-red-300 text-sm"
              >
                Remove
              </button>
            </div>
          ))}
        </div>
      )}

      {!showAdd ? (
        <button
          onClick={() => setShowAdd(true)}
          className="w-full px-3 py-2 text-sm text-blue-600 dark:text-blue-400 bg-blue-50 dark:bg-blue-900/20 hover:bg-blue-100 dark:hover:bg-blue-900/30 rounded border border-blue-200 dark:border-blue-700 transition-colors"
        >
          + Add Model
        </button>
      ) : (
        <div className="flex gap-2">
          <select
            onChange={(e) => {
              const [provider, model] = e.target.value.split('/');
              setSelectedModel([provider, model]);
            }}
            className="flex-1 p-2 text-sm border border-gray-300 dark:border-gray-600 rounded bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100"
          >
            <option value="">Select a model...</option>
            {availableModels.map(([provider, model], idx) => (
              <option key={idx} value={`${provider}/${model}`}>
                {provider}/{model}
              </option>
            ))}
          </select>
          <button
            onClick={() => {
              if (selectedModel) {
                onAdd(selectedModel[0], selectedModel[1]);
                setShowAdd(false);
                setSelectedModel(null);
              }
            }}
            disabled={!selectedModel}
            className="px-4 py-2 text-sm bg-blue-600 text-white rounded hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700 disabled:bg-gray-400 dark:disabled:bg-gray-600 disabled:cursor-not-allowed transition-colors"
          >
            Add
          </button>
          <button
            onClick={() => {
              setShowAdd(false);
              setSelectedModel(null);
            }}
            className="px-4 py-2 text-sm bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded hover:bg-gray-300 dark:hover:bg-gray-600 transition-colors"
          >
            Cancel
          </button>
        </div>
      )}
    </div>
  );
};
