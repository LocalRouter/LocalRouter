import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-shell';

interface McpOAuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  serverId: string;
  serverName: string;
  onSuccess: () => void;
}

interface OAuthBrowserFlowResult {
  auth_url: string;
  redirect_uri: string;
  state: string;
}

type OAuthBrowserFlowStatus =
  | { type: 'Pending' }
  | { type: 'Success'; expires_in: number }
  | { type: 'Error'; message: string }
  | { type: 'Timeout' };

export const McpOAuthModal: React.FC<McpOAuthModalProps> = ({
  isOpen,
  onClose,
  serverId,
  serverName,
  onSuccess,
}) => {
  const [flowResult, setFlowResult] = useState<OAuthBrowserFlowResult | null>(null);
  const [flowStatus, setFlowStatus] = useState<OAuthBrowserFlowStatus | null>(null);
  const [isPolling, setIsPolling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [browserOpened, setBrowserOpened] = useState(false);

  // Start OAuth flow when modal opens
  useEffect(() => {
    if (isOpen && !flowResult) {
      startOAuthFlow();
    }
  }, [isOpen]);

  // Auto-open browser when auth URL is ready
  useEffect(() => {
    if (flowResult && !browserOpened) {
      openBrowser();
    }
  }, [flowResult, browserOpened]);

  // Poll for OAuth completion
  useEffect(() => {
    if (!isPolling) {
      return;
    }

    const pollInterval = setInterval(async () => {
      try {
        const status = await invoke<OAuthBrowserFlowStatus>('poll_mcp_oauth_browser_status', {
          serverId,
        });

        setFlowStatus(status);

        if (status.type === 'Success') {
          setIsPolling(false);
          setTimeout(() => {
            onSuccess();
            handleClose();
          }, 1500);
        } else if (status.type === 'Error') {
          setIsPolling(false);
          setError(status.message);
        } else if (status.type === 'Timeout') {
          setIsPolling(false);
          setError('Authentication timeout (5 minutes). Please try again.');
        }
      } catch (err) {
        console.error('Failed to poll OAuth status:', err);
        setError(String(err));
        setIsPolling(false);
      }
    }, 2000); // Poll every 2 seconds (faster than provider OAuth)

    return () => clearInterval(pollInterval);
  }, [isPolling, serverId]);

  const startOAuthFlow = async () => {
    setError(null);
    setBrowserOpened(false);
    try {
      const result = await invoke<OAuthBrowserFlowResult>('start_mcp_oauth_browser_flow', {
        serverId,
      });
      setFlowResult(result);
      setFlowStatus({ type: 'Pending' });
      setIsPolling(true);
    } catch (err) {
      console.error('Failed to start OAuth flow:', err);
      setError(String(err));
    }
  };

  const openBrowser = async () => {
    if (!flowResult?.auth_url) return;

    try {
      await open(flowResult.auth_url);
      setBrowserOpened(true);
    } catch (err) {
      console.error('Failed to open browser:', err);
      // Not a fatal error - user can manually open the URL
    }
  };

  const handleClose = async () => {
    if (isPolling) {
      try {
        await invoke('cancel_mcp_oauth_browser_flow', { serverId });
      } catch (err) {
        console.error('Failed to cancel OAuth flow:', err);
      }
      setIsPolling(false);
    }
    setFlowResult(null);
    setFlowStatus(null);
    setError(null);
    setBrowserOpened(false);
    onClose();
  };

  const handleOpenUrl = async () => {
    if (flowResult?.auth_url) {
      try {
        await open(flowResult.auth_url);
      } catch (err) {
        console.error('Failed to open URL:', err);
      }
    }
  };

  const handleCopyUrl = () => {
    if (flowResult?.auth_url) {
      navigator.clipboard.writeText(flowResult.auth_url);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black bg-opacity-50 dark:bg-opacity-70">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-xl max-w-md w-full p-6">
        <div className="flex justify-between items-center mb-4">
          <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
            Authenticate with {serverName}
          </h2>
          <button
            onClick={handleClose}
            className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
          >
            ✕
          </button>
        </div>

        {error && (
          <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-400 px-4 py-3 rounded mb-4">
            <p className="font-semibold mb-1">Error</p>
            <p className="text-sm">{error}</p>
          </div>
        )}

        {flowStatus?.type === 'Pending' && (
          <div className="space-y-4">
            <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-4">
              <p className="text-sm text-blue-700 dark:text-blue-300 mb-2">
                A browser window should have opened automatically. If not, click the button below:
              </p>
              <button
                onClick={handleOpenUrl}
                className="w-full px-4 py-2 bg-blue-500 dark:bg-blue-600 text-white rounded hover:bg-blue-600 dark:hover:bg-blue-700"
              >
                Open Browser
              </button>
            </div>

            {flowResult && (
              <div className="bg-gray-50 dark:bg-gray-700 border border-gray-200 dark:border-gray-600 rounded p-3">
                <div className="text-xs text-gray-600 dark:text-gray-400 mb-1">
                  Authorization URL:
                </div>
                <div className="flex items-center space-x-2">
                  <code className="text-xs font-mono text-gray-700 dark:text-gray-300 break-all flex-1">
                    {flowResult.auth_url.substring(0, 60)}...
                  </code>
                  <button
                    onClick={handleCopyUrl}
                    className="px-2 py-1 text-xs bg-gray-200 dark:bg-gray-600 rounded hover:bg-gray-300 dark:hover:bg-gray-500"
                    title="Copy URL"
                  >
                    Copy
                  </button>
                </div>
                <div className="text-xs text-gray-500 dark:text-gray-400 mt-2">
                  Redirect URI: {flowResult.redirect_uri}
                </div>
              </div>
            )}

            <div className="flex items-center justify-center space-x-2 text-gray-500 dark:text-gray-400">
              <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-blue-500 dark:border-blue-400"></div>
              <span>Waiting for authorization...</span>
            </div>

            <div className="text-xs text-gray-500 dark:text-gray-400 text-center">
              Complete the authentication in your browser, then return here.
            </div>
          </div>
        )}

        {flowStatus?.type === 'Success' && (
          <div className="text-center space-y-4">
            <div className="text-green-500 dark:text-green-400 text-5xl">✓</div>
            <p className="text-lg font-semibold text-gray-700 dark:text-gray-300">
              Authentication Successful!
            </p>
            <p className="text-gray-600 dark:text-gray-400">
              Your {serverName} MCP server has been authenticated.
            </p>
            {flowStatus.expires_in && (
              <p className="text-xs text-gray-500 dark:text-gray-400">
                Token expires in {Math.floor(flowStatus.expires_in / 3600)} hours
              </p>
            )}
          </div>
        )}

        {!flowResult && !error && (
          <div className="flex items-center justify-center py-8">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500 dark:border-blue-400"></div>
          </div>
        )}
      </div>
    </div>
  );
};
