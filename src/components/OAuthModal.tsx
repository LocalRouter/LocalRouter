import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface OAuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  providerId: string;
  providerName: string;
  onSuccess: () => void;
}

interface OAuthFlowResult {
  type: 'pending' | 'success' | 'error';
  user_code?: string;
  verification_url?: string;
  instructions?: string;
  message?: string;
}

export const OAuthModal: React.FC<OAuthModalProps> = ({
  isOpen,
  onClose,
  providerId,
  providerName,
  onSuccess,
}) => {
  const [flowResult, setFlowResult] = useState<OAuthFlowResult | null>(null);
  const [isPolling, setIsPolling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Start OAuth flow when modal opens
  useEffect(() => {
    if (isOpen && !flowResult) {
      startOAuthFlow();
    }
  }, [isOpen]);

  // Poll for OAuth completion
  useEffect(() => {
    if (!isPolling || !flowResult || flowResult.type !== 'pending') {
      return;
    }

    const pollInterval = setInterval(async () => {
      try {
        const result = await invoke<OAuthFlowResult>('poll_oauth_status', {
          providerId,
        });

        setFlowResult(result);

        if (result.type === 'success') {
          setIsPolling(false);
          setTimeout(() => {
            onSuccess();
            handleClose();
          }, 1500);
        } else if (result.type === 'error') {
          setIsPolling(false);
          setError(result.message || 'Authentication failed');
        }
      } catch (err) {
        console.error('Failed to poll OAuth status:', err);
        setError(String(err));
        setIsPolling(false);
      }
    }, 5000); // Poll every 5 seconds

    return () => clearInterval(pollInterval);
  }, [isPolling, flowResult, providerId]);

  const startOAuthFlow = async () => {
    setError(null);
    try {
      const result = await invoke<OAuthFlowResult>('start_oauth_flow', {
        providerId,
      });
      setFlowResult(result);

      if (result.type === 'pending') {
        setIsPolling(true);
      } else if (result.type === 'error') {
        setError(result.message || 'Failed to start OAuth flow');
      }
    } catch (err) {
      console.error('Failed to start OAuth flow:', err);
      setError(String(err));
    }
  };

  const handleClose = async () => {
    if (isPolling) {
      try {
        await invoke('cancel_oauth_flow', { providerId });
      } catch (err) {
        console.error('Failed to cancel OAuth flow:', err);
      }
      setIsPolling(false);
    }
    setFlowResult(null);
    setError(null);
    onClose();
  };

  const handleOpenUrl = () => {
    if (flowResult?.verification_url) {
      window.open(flowResult.verification_url, '_blank');
    }
  };

  const handleCopyCode = () => {
    if (flowResult?.user_code) {
      navigator.clipboard.writeText(flowResult.user_code);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black bg-opacity-50">
      <div className="bg-white rounded-lg shadow-xl max-w-md w-full p-6">
        <div className="flex justify-between items-center mb-4">
          <h2 className="text-xl font-semibold">
            Authenticate with {providerName}
          </h2>
          <button
            onClick={handleClose}
            className="text-gray-500 hover:text-gray-700"
          >
            ✕
          </button>
        </div>

        {error && (
          <div className="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded mb-4">
            {error}
          </div>
        )}

        {flowResult?.type === 'pending' && (
          <div className="space-y-4">
            <p className="text-gray-700">{flowResult.instructions}</p>

            {flowResult.user_code && (
              <div className="bg-gray-50 border border-gray-200 rounded p-4">
                <div className="text-sm text-gray-600 mb-2">
                  Verification Code:
                </div>
                <div className="flex items-center justify-between">
                  <code className="text-2xl font-mono font-bold">
                    {flowResult.user_code}
                  </code>
                  <button
                    onClick={handleCopyCode}
                    className="px-3 py-1 bg-blue-500 text-white rounded hover:bg-blue-600"
                  >
                    Copy
                  </button>
                </div>
              </div>
            )}

            {flowResult.verification_url && (
              <button
                onClick={handleOpenUrl}
                className="w-full px-4 py-2 bg-blue-500 text-white rounded hover:bg-blue-600"
              >
                Open Authentication Page
              </button>
            )}

            <div className="flex items-center justify-center space-x-2 text-gray-500">
              <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-blue-500"></div>
              <span>Waiting for authorization...</span>
            </div>
          </div>
        )}

        {flowResult?.type === 'success' && (
          <div className="text-center space-y-4">
            <div className="text-green-500 text-5xl">✓</div>
            <p className="text-lg font-semibold text-gray-700">
              Authentication Successful!
            </p>
            <p className="text-gray-600">
              Your {providerName} account has been connected.
            </p>
          </div>
        )}

        {!flowResult && !error && (
          <div className="flex items-center justify-center py-8">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
          </div>
        )}
      </div>
    </div>
  );
};
