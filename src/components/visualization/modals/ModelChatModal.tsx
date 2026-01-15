import { useState, useEffect } from 'react';
import Modal from '../../ui/Modal';
import { ChatInterface } from '../ChatInterface';
import { invoke } from '@tauri-apps/api/core';
import OpenAI from 'openai';

interface ModelChatModalProps {
  isOpen: boolean;
  onClose: () => void;
  modelData: {
    model_id: string;
    provider_instance: string;
    capabilities: string[];
    context_window: number;
    supports_streaming: boolean;
    label?: string;
  };
}

export function ModelChatModal({ isOpen, onClose, modelData }: ModelChatModalProps) {
  const [client, setClient] = useState<OpenAI | null>(null);
  const [apiKey, setApiKey] = useState<string>('');

  useEffect(() => {
    if (isOpen) {
      loadServerConfig();
    }
  }, [isOpen]);

  const loadServerConfig = async () => {
    try {
      const config = await invoke<{ host: string; port: number }>('get_server_config');

      // Get the first available API key for authentication
      const keys = await invoke<Array<{ id: string; enabled: boolean }>>('list_api_keys');
      const enabledKey = keys.find((k) => k.enabled);
      if (enabledKey) {
        const keyValue = await invoke<string>('get_api_key_value', { id: enabledKey.id });
        setApiKey(keyValue);

        // Initialize OpenAI client
        const newClient = new OpenAI({
          apiKey: keyValue,
          baseURL: `http://${config.host}:${config.port}/v1`,
          dangerouslyAllowBrowser: true,
        });
        setClient(newClient);
      }
    } catch (err) {
      console.error('Failed to load server config:', err);
    }
  };

  const handleSendMessage = async (
    messages: Array<{ role: 'user' | 'assistant'; content: string }>,
    userMessage: string
  ) => {
    if (!client) {
      throw new Error('Chat client not initialized');
    }

    const stream = await client.chat.completions.create({
      model: `${modelData.provider_instance}/${modelData.model_id}`,
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
  const formatContextWindow = (tokens: number) => {
    if (tokens >= 1000000) {
      return `${(tokens / 1000000).toFixed(1)}M tokens`;
    } else if (tokens >= 1000) {
      return `${(tokens / 1000).toFixed(0)}K tokens`;
    }
    return `${tokens} tokens`;
  };

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={`Model: ${modelData.label || modelData.model_id}`}
    >
      <div className="space-y-4">
        {/* Model Details */}
        <div className="bg-purple-50 border border-purple-200 rounded-lg p-4">
          <h3 className="font-semibold text-purple-900 mb-3">Model Details</h3>
          <div className="space-y-2 text-sm">
            <div className="flex justify-between">
              <span className="text-gray-600">Provider:</span>
              <span className="font-medium text-gray-900">{modelData.provider_instance}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">Model ID:</span>
              <span className="font-medium text-gray-900">{modelData.model_id}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">Context Window:</span>
              <span className="font-medium text-gray-900">
                {formatContextWindow(modelData.context_window)}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-600">Streaming:</span>
              <span className="font-medium text-gray-900">
                {modelData.supports_streaming ? 'Yes' : 'No'}
              </span>
            </div>
            {modelData.capabilities.length > 0 && (
              <div className="flex justify-between">
                <span className="text-gray-600">Capabilities:</span>
                <span className="font-medium text-gray-900">
                  {modelData.capabilities.join(', ')}
                </span>
              </div>
            )}
          </div>
        </div>

        {/* Chat Interface */}
        <div>
          {client && apiKey ? (
            <ChatInterface
              onSendMessage={handleSendMessage}
              placeholder={`Chat with ${modelData.label || modelData.model_id}...`}
            />
          ) : (
            <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4">
              <p className="text-yellow-900 text-sm">
                <strong>Note:</strong> To use chat, make sure the server is running and you
                have at least one enabled API key.
              </p>
            </div>
          )}
        </div>

        {/* Close Button */}
        <div className="flex justify-end">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-gray-200 text-gray-700 rounded-lg hover:bg-gray-300 transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </Modal>
  );
}
