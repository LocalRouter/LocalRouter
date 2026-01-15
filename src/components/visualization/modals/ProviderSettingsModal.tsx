import { useState, useEffect } from 'react';
import Modal from '../../ui/Modal';
import { invoke } from '@tauri-apps/api/core';
import Button from '../../ui/Button';

interface ProviderSettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  providerData: {
    instance_name: string;
    provider_type: string;
    enabled: boolean;
  };
  onUpdate?: () => void;
}

export function ProviderSettingsModal({
  isOpen,
  onClose,
  providerData,
  onUpdate,
}: ProviderSettingsModalProps) {
  const [config, setConfig] = useState<Record<string, string>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [enabled, setEnabled] = useState(providerData.enabled);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    if (isOpen) {
      loadProviderConfig();
    }
  }, [isOpen, providerData.instance_name]);

  const loadProviderConfig = async () => {
    try {
      setIsLoading(true);
      const providerConfig = await invoke<Record<string, string>>('get_provider_config', {
        instanceName: providerData.instance_name,
      });
      setConfig(providerConfig);
      setEnabled(providerData.enabled);
    } catch (err) {
      console.error('Failed to load provider config:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleSave = async () => {
    try {
      setIsSaving(true);

      // Update enabled status if changed
      if (enabled !== providerData.enabled) {
        await invoke('set_provider_enabled', {
          instanceName: providerData.instance_name,
          enabled,
        });
      }

      if (onUpdate) {
        onUpdate();
      }
      onClose();
    } catch (err) {
      console.error('Failed to update provider:', err);
      alert(`Failed to update provider: ${err}`);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={`Provider: ${providerData.instance_name}`}>
      {isLoading ? (
        <div className="flex items-center justify-center py-8">
          <div className="text-gray-600">Loading...</div>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
            <h3 className="font-semibold text-blue-900 mb-3">Provider Details</h3>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-600">Name:</span>
                <span className="font-medium text-gray-900">
                  {providerData.instance_name}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600">Type:</span>
                <span className="font-medium text-gray-900">{providerData.provider_type}</span>
              </div>
            </div>
          </div>

          <div>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
                className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500"
              />
              <span className="text-sm font-medium text-gray-700">Enabled</span>
            </label>
          </div>

          {Object.keys(config).length > 0 && (
            <div className="bg-gray-50 border border-gray-200 rounded-lg p-4">
              <h3 className="font-semibold text-gray-900 mb-2 text-sm">Configuration</h3>
              <div className="space-y-1 text-xs">
                {Object.entries(config).map(([key, value]) => (
                  <div key={key} className="flex justify-between">
                    <span className="text-gray-600">{key}:</span>
                    <span className="font-mono text-gray-900">
                      {value.includes('://') || value.length > 30
                        ? `${value.substring(0, 30)}...`
                        : value}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}

          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" onClick={handleSave} disabled={isSaving}>
              {isSaving ? 'Saving...' : 'Save Changes'}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
