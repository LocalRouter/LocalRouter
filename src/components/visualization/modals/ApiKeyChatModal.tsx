import { useState, useEffect } from 'react';
import Modal from '../../ui/Modal';
import { ChatInterface } from '../ChatInterface';
import { invoke } from '@tauri-apps/api/core';
import OpenAI from 'openai';

interface ApiKeyChatModalProps {
  isOpen: boolean;
  onClose: () => void;
  apiKeyData: {
    key_id: string;
    key_name: string;
    enabled: boolean;
    created_at: string;
    routing_strategy: string | null;
  };
  onUpdate?: () => void;
}

export function ApiKeyChatModal({
  isOpen,
  onClose,
  apiKeyData,
  onUpdate,
}: ApiKeyChatModalProps) {
  const [activeTab, setActiveTab] = useState<'chat' | 'settings'>('chat');
  const [name, setName] = useState(apiKeyData.key_name);
  const [enabled, setEnabled] = useState(apiKeyData.enabled);
  const [isSaving, setIsSaving] = useState(false);
  const [client, setClient] = useState<OpenAI | null>(null);
  const [apiKey, setApiKey] = useState<string>('');

  useEffect(() => {
    if (isOpen) {
      loadClient();
    }
  }, [isOpen, apiKeyData.key_id]);

  const loadClient = async () => {
    try {
      const config = await invoke<{ host: string; port: number }>('get_server_config');
      const keyValue = await invoke<string>('get_api_key_value', { id: apiKeyData.key_id });
      setApiKey(keyValue);

      const newClient = new OpenAI({
        apiKey: keyValue,
        baseURL: `http://${config.host}:${config.port}/v1`,
        dangerouslyAllowBrowser: true,
      });
      setClient(newClient);
    } catch (err) {
      console.error('Failed to load API key:', err);
    }
  };

  const handleSendMessage = async (
    messages: Array<{ role: 'user' | 'assistant'; content: string }>,
    userMessage: string
  ) => {
    if (!client) {
      throw new Error('Chat client not initialized');
    }

    // Use 'gpt-4' as a generic model identifier - routing will handle the actual model
    const stream = await client.chat.completions.create({
      model: 'gpt-4',
      messages: [
        ...messages,
        {
          role: 'user',
          content: userMessage,
        },
      ],
      stream: true,
    });

    // Return async generator for streaming
    async function* generateChunks() {
      for await (const chunk of stream) {
        const content = chunk.choices[0]?.delta?.content || '';
        if (content) {
          yield content;
        }
      }
    }

    return generateChunks();
  };

  const handleSaveSettings = async () => {
    try {
      setIsSaving(true);

      // Update name if changed
      if (name !== apiKeyData.key_name) {
        await invoke('update_api_key_name', {
          id: apiKeyData.key_id,
          name,
        });
      }

      // Update enabled status if changed
      if (enabled !== apiKeyData.enabled) {
        await invoke('toggle_api_key_enabled', {
          id: apiKeyData.key_id,
          enabled,
        });
      }

      if (onUpdate) {
        onUpdate();
      }
      onClose();
    } catch (err) {
      console.error('Failed to update API key:', err);
      alert(`Failed to update API key: ${err}`);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={`API Key: ${apiKeyData.key_name}`}>
      <div className="space-y-4">
        {/* Tabs */}
        <div className="flex border-b border-gray-200">
          <button
            onClick={() => setActiveTab('chat')}
            className={`px-4 py-2 font-medium transition-colors ${
              activeTab === 'chat'
                ? 'border-b-2 border-blue-500 text-blue-600'
                : 'text-gray-600 hover:text-gray-900'
            }`}
          >
            Chat
          </button>
          <button
            onClick={() => setActiveTab('settings')}
            className={`px-4 py-2 font-medium transition-colors ${
              activeTab === 'settings'
                ? 'border-b-2 border-blue-500 text-blue-600'
                : 'text-gray-600 hover:text-gray-900'
            }`}
          >
            Settings
          </button>
        </div>

        {/* Tab Content */}
        {activeTab === 'chat' ? (
          <div>
            {client && apiKey ? (
              <div>
                <div className="mb-2 text-xs text-gray-600">
                  Routing: {apiKeyData.routing_strategy || 'Default'}
                </div>
                <ChatInterface
                  onSendMessage={handleSendMessage}
                  placeholder={`Chat using ${apiKeyData.key_name}...`}
                  disabled={!enabled}
                />
                {!enabled && (
                  <div className="mt-2 text-xs text-red-600">
                    This API key is disabled. Enable it in Settings to chat.
                  </div>
                )}
              </div>
            ) : (
              <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4">
                <p className="text-yellow-900 text-sm">
                  <strong>Note:</strong> To use chat, make sure the server is running.
                </p>
              </div>
            )}
          </div>
        ) : (
          <div className="space-y-4">
            {/* Settings Form */}
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Name
              </label>
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
              />
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

            <div className="bg-blue-50 border border-blue-200 rounded-lg p-3">
              <div className="text-xs text-blue-900">
                <div className="mb-1">
                  <span className="font-semibold">Created:</span>{' '}
                  {new Date(apiKeyData.created_at).toLocaleDateString()}
                </div>
                <div>
                  <span className="font-semibold">Routing:</span>{' '}
                  {apiKeyData.routing_strategy || 'Not configured'}
                </div>
              </div>
            </div>

            {/* Action Buttons */}
            <div className="flex justify-end gap-2">
              <button
                onClick={onClose}
                className="px-4 py-2 bg-gray-200 text-gray-700 rounded-lg hover:bg-gray-300 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleSaveSettings}
                disabled={isSaving}
                className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
              >
                {isSaving ? 'Saving...' : 'Save Changes'}
              </button>
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}
