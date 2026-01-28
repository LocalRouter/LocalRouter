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
  const [retryCount, setRetryCount] = useState(0);
  const [availableModels, setAvailableModels] = useState<Array<{ model_id: string; provider_instance: string }>>([]);
  const [loadingModels, setLoadingModels] = useState(false);

  // Fetch available models for API key context by calling the web server
  const fetchModels = async () => {
    if (context.type !== 'api_key') return;
    if (!chatClient) {
      console.log('fetchModels: chatClient not ready yet');
      return;
    }

    console.log('fetchModels: Calling /v1/models endpoint');
    setLoadingModels(true);
    try {
      // Call the /v1/models endpoint using the OpenAI client
      const response = await chatClient.models.list();
      console.log('fetchModels: Success, received models:', response.data.length);
      const formattedModels = response.data.map(m => ({
        model_id: m.id,
        provider_instance: m.id.split('/')[0] || 'unknown'
      }));
      setAvailableModels(formattedModels);

      // Set first model as default if none selected
      if (!selectedModel && formattedModels.length > 0) {
        setSelectedModel(formattedModels[0].model_id);
      }
    } catch (err: any) {
      console.error('fetchModels: Failed with error:', err);
      console.error('fetchModels: Error details:', {
        message: err.message,
        status: err.status,
        response: err.response
      });
      setError(`Failed to fetch models: ${err.message || err}`);
    } finally {
      setLoadingModels(false);
    }
  };

  // Initialize model selection based on context
  useEffect(() => {
    if (context.type === 'provider') {
      setSelectedModel(context.models[0]?.model_id || null);
    } else if (context.type === 'model') {
      setSelectedModel(context.modelId);
    } else if (context.type === 'api_key') {
      // Fetch available models for API key context when chat client is ready
      if (chatClient) {
        fetchModels();
      }
    }
  }, [context, chatClient]);

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

        let apiKeyValue: string | undefined;

        if (context.type === 'api_key') {
          // If bearer token is provided directly, use it (for unified clients)
          if (context.bearerToken) {
            console.log('Using provided bearer token');
            apiKeyValue = context.bearerToken;
          } else {
            console.log('Fetching bearer token for client:', context.apiKeyId);
            // Use unified client manager to get the client secret
            try {
              console.log('Calling get_client_value...');
              apiKeyValue = await invoke<string>('get_client_value', { id: context.apiKeyId });
              console.log('get_client_value succeeded, secret length:', apiKeyValue?.length);
            } catch (err: any) {
              const errorMsg = err?.toString() || '';
              console.error('Failed to load client secret:', errorMsg);

              // Check if it's a missing secret issue
              if (errorMsg.includes('not found in keychain')) {
                throw new Error(
                  'Client secret not found. This client may have been created before secret storage was implemented. ' +
                  'Please create a new client or contact support to regenerate the secret.'
                );
              }

              // Show the error
              throw new Error(`Failed to load client token: ${errorMsg}`);
            }
          }
        } else {
          // For provider/model context, use internal testing mode (bypasses API key restrictions)
          // Fetch the transient internal test bearer token (only accessible via Tauri IPC)
          try {
            apiKeyValue = await invoke<string>('get_internal_test_token');
          } catch (tokenErr: any) {
            throw new Error(`Failed to get internal test token: ${tokenErr}`);
          }
        }

        // Ensure apiKeyValue is set
        if (!apiKeyValue) {
          throw new Error('Failed to obtain API key value');
        }

        const port = serverConfig.actual_port ?? serverConfig.port;
        const baseURL = `http://${serverConfig.host}:${port}/v1`;

        console.log('Initializing OpenAI client:', {
          baseURL,
          apiKeyLength: apiKeyValue.length,
          apiKeyPrefix: apiKeyValue.substring(0, 10) + '...',
          contextType: context.type
        });

        const client = new OpenAI({
          apiKey: apiKeyValue,
          baseURL,
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
  }, [context, retryCount]);

  // Get model string based on context
  const getModelString = (): string => {
    switch (context.type) {
      case 'api_key':
        // Use selected model if available, otherwise fallback to 'gpt-4'
        return selectedModel || 'gpt-4';
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

  // Function to retry initialization
  const retryInitialization = () => {
    setError(null);
    setLoading(true);
    setChatClient(null);
    // Increment retry count to trigger useEffect
    setRetryCount(prev => prev + 1);
  };

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
          onClick={retryInitialization}
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700 transition-colors"
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
          <div className="flex items-center gap-2 text-sm text-blue-800 dark:text-blue-300">
            <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 20 20">
              <path
                fillRule="evenodd"
                d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                clipRule="evenodd"
              />
            </svg>
            <span className="font-medium">
              Selection enabled - The actual model used may differ based on your strategy rules
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

      {/* Model Selector for API Key Context */}
      {context.type === 'api_key' && (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <label className="block text-sm font-medium text-gray-900 dark:text-gray-100">Select Model</label>
            <button
              onClick={fetchModels}
              disabled={loadingModels || disabled}
              className="px-3 py-1 text-sm bg-blue-600 text-white rounded hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {loadingModels ? 'Refreshing...' : 'Refresh Models'}
            </button>
          </div>
          {availableModels.length > 0 ? (
            <ModelSelector
              models={availableModels}
              selectedModel={selectedModel}
              onModelChange={setSelectedModel}
              disabled={disabled}
              label=""
            />
          ) : (
            <div className="p-3 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg">
              <p className="text-sm text-yellow-800 dark:text-yellow-400">
                Loading models...
              </p>
            </div>
          )}
        </div>
      )}

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

      {/* Note about persistence */}
      <div className="mt-3 px-3 py-2 rounded-lg bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700">
        <p className="text-xs text-gray-600 dark:text-gray-400">
          Note: Chat conversations are not persisted and will be lost when you navigate away or refresh the page.
        </p>
      </div>
    </div>
  );
};
