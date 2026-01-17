import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import OpenAI from 'openai';
import { ChatInterface } from '../visualization/ChatInterface';
import { ModelSelector } from './ModelSelector';
import { ModelUsedDisplay } from './ModelUsedDisplay';
import { ContextualChatProps } from './types';

export const ContextualChat: React.FC<ContextualChatProps> = ({ context, disabled = false }) => {
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [actualModelUsed, setActualModelUsed] = useState<string | null>(null);
  const [chatClient, setChatClient] = useState<OpenAI | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  // Initialize model selection based on context
  useEffect(() => {
    if (context.type === 'provider') {
      setSelectedModel(context.models[0]?.model_id || null);
    } else if (context.type === 'model') {
      setSelectedModel(context.modelId);
    } else {
      // API key context uses generic 'gpt-4'
      setSelectedModel('gpt-4');
    }
  }, [context]);

  // Load server config and create OpenAI client
  useEffect(() => {
    const initializeClient = async () => {
      setLoading(true);
      setError(null);

      try {
        // Get server configuration
        const serverConfig = await invoke<{ host: string; port: number; actual_port?: number }>(
          'get_server_config'
        );

        let apiKeyValue: string;

        if (context.type === 'api_key') {
          // For API key context, use the provided key
          try {
            apiKeyValue = await invoke<string>('get_api_key_value', { id: context.apiKeyId });
          } catch (keyErr: any) {
            const errorMsg = keyErr?.toString() || 'Unknown error';
            if (errorMsg.includes('passphrase') || errorMsg.includes('keychain')) {
              throw new Error('Keychain access required to use chat. Please approve keychain access.');
            }
            throw new Error(`Failed to load API key: ${errorMsg}`);
          }
        } else {
          // For provider/model context, get first enabled API key
          const allKeys = await invoke<Array<{ id: string; enabled: boolean }>>('list_api_keys');
          const enabledKey = allKeys.find((k) => k.enabled);

          if (!enabledKey) {
            throw new Error('No enabled API key available. Create and enable an API key first.');
          }

          try {
            apiKeyValue = await invoke<string>('get_api_key_value', { id: enabledKey.id });
          } catch (keyErr: any) {
            const errorMsg = keyErr?.toString() || 'Unknown error';
            if (errorMsg.includes('passphrase') || errorMsg.includes('keychain')) {
              throw new Error('Keychain access required to use chat. Please approve keychain access.');
            }
            throw new Error(`Failed to load API key: ${errorMsg}`);
          }
        }

        const port = serverConfig.actual_port ?? serverConfig.port;
        const client = new OpenAI({
          apiKey: apiKeyValue,
          baseURL: `http://${serverConfig.host}:${port}/v1`,
          dangerouslyAllowBrowser: true,
        });

        setChatClient(client);
      } catch (err: any) {
        console.error('Failed to initialize chat client:', err);
        setError(err.message || 'Failed to initialize chat');
      } finally {
        setLoading(false);
      }
    };

    initializeClient();
  }, [context]);

  // Get model string based on context
  const getModelString = (): string => {
    switch (context.type) {
      case 'api_key':
        return 'gpt-4'; // Generic, routing decides
      case 'provider':
        return selectedModel ? `${context.instanceName}/${selectedModel}` : '';
      case 'model':
        return `${context.providerInstance}/${context.modelId}`;
    }
  };

  // Handle sending message with model capture
  const handleSendMessage = async (
    messages: Array<{ role: 'user' | 'assistant'; content: string }>,
    userMessage: string
  ) => {
    if (!chatClient) {
      throw new Error('Chat client not initialized');
    }

    const modelString = getModelString();
    if (!modelString) {
      throw new Error('No model selected');
    }

    // Reset actual model for new message
    setActualModelUsed(null);

    const stream = await chatClient.chat.completions.create({
      model: modelString,
      messages: [
        ...messages,
        {
          role: 'user',
          content: userMessage,
        },
      ],
      stream: true,
    });

    let modelCaptured = false;

    async function* generateChunks() {
      for await (const chunk of stream) {
        // CAPTURE ACTUAL MODEL FROM FIRST CHUNK
        if (chunk.model && !modelCaptured) {
          setActualModelUsed(chunk.model);
          modelCaptured = true;
        }

        const content = chunk.choices[0]?.delta?.content || '';
        if (content) {
          yield content;
        }
      }
    }

    return generateChunks();
  };

  // Render loading state
  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <div className="text-gray-500 dark:text-gray-400">Initializing chat...</div>
      </div>
    );
  }

  // Render error state
  if (error) {
    return (
      <div className="space-y-4">
        <div className="p-4 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
          <div className="flex items-start gap-3">
            <svg
              className="w-5 h-5 text-red-600 dark:text-red-400 flex-shrink-0 mt-0.5"
              fill="currentColor"
              viewBox="0 0 20 20"
            >
              <path
                fillRule="evenodd"
                d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z"
                clipRule="evenodd"
              />
            </svg>
            <div className="flex-1">
              <h3 className="text-sm font-medium text-red-800 dark:text-red-300">Chat Error</h3>
              <p className="mt-1 text-sm text-red-700 dark:text-red-400">{error}</p>
            </div>
          </div>
        </div>
        <button
          onClick={() => window.location.reload()}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
        >
          Retry
        </button>
      </div>
    );
  }

  // Render no models warning for provider context
  if (context.type === 'provider' && context.models.length === 0) {
    return (
      <div className="p-4 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg">
        <div className="flex items-start gap-3">
          <svg
            className="w-5 h-5 text-yellow-600 dark:text-yellow-400 flex-shrink-0 mt-0.5"
            fill="currentColor"
            viewBox="0 0 20 20"
          >
            <path
              fillRule="evenodd"
              d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z"
              clipRule="evenodd"
            />
          </svg>
          <div>
            <h3 className="text-sm font-medium text-yellow-800 dark:text-yellow-300">
              No Models Available
            </h3>
            <p className="mt-1 text-sm text-yellow-700 dark:text-yellow-400">
              No models are available for this provider. Please check the provider configuration.
            </p>
          </div>
        </div>
      </div>
    );
  }

  // Render context info badge
  const renderContextInfo = () => {
    if (context.type === 'api_key') {
      return (
        <div className="mb-3 p-3 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg">
          <div className="flex items-center gap-2 text-sm text-blue-700 dark:text-blue-300">
            <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 20 20">
              <path
                fillRule="evenodd"
                d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                clipRule="evenodd"
              />
            </svg>
            <span className="font-medium">
              Routing enabled - The actual model used may differ based on your routing rules
            </span>
          </div>
        </div>
      );
    } else if (context.type === 'provider') {
      return (
        <div className="mb-3 flex items-center gap-2 px-3 py-2 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg">
          <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300">
            Direct Provider Access
          </span>
        </div>
      );
    } else {
      return (
        <div className="mb-3 flex items-center gap-2 px-3 py-2 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg">
          <span className="inline-flex items-center px-2 py-1 rounded text-xs font-medium bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300">
            Direct Model Access
          </span>
        </div>
      );
    }
  };

  return (
    <div className="space-y-4">
      {renderContextInfo()}

      {/* Model Selector for Provider Context */}
      {context.type === 'provider' && (
        <ModelSelector
          models={context.models}
          selectedModel={selectedModel}
          onModelChange={setSelectedModel}
          disabled={disabled}
          label="Select Model"
        />
      )}

      {/* Chat Interface */}
      <ChatInterface
        onSendMessage={handleSendMessage}
        placeholder="Type your message..."
        disabled={disabled || !chatClient}
      />

      {/* Display Actual Model Used */}
      {actualModelUsed && (
        <ModelUsedDisplay
          requestedModel={getModelString()}
          actualModel={actualModelUsed}
          contextType={context.type}
        />
      )}
    </div>
  );
};
