/**
 * RouteLLM Status Indicator Component
 * Displays the current status of the RouteLLM service with beta badge
 */

import React from 'react';
import { RouteLLMStatus, RouteLLMState } from './types';

interface RouteLLMStatusIndicatorProps {
  status: RouteLLMStatus;
  compact?: boolean;
  modelPath?: string;
  onOpenFolder?: () => void;
  onDownload?: () => void;
  onUnload?: () => void;
  isDownloading?: boolean;
}

interface StatusConfig {
  color: string;
  bgColor: string;
  icon: string;
  label: string;
}

const getStatusConfig = (state: RouteLLMState): StatusConfig => {
  switch (state) {
    case 'not_downloaded':
      return {
        color: 'text-gray-700 dark:text-gray-300',
        bgColor: 'bg-gray-100 dark:bg-gray-800',
        icon: '⬇️',
        label: 'Not Downloaded',
      };
    case 'downloading':
      return {
        color: 'text-blue-700 dark:text-blue-300',
        bgColor: 'bg-blue-50 dark:bg-blue-900/20',
        icon: '⏳',
        label: 'Downloading...',
      };
    case 'downloaded_not_running':
      return {
        color: 'text-yellow-700 dark:text-yellow-300',
        bgColor: 'bg-yellow-50 dark:bg-yellow-900/20',
        icon: '⏸️',
        label: 'Ready',
      };
    case 'initializing':
      return {
        color: 'text-orange-700 dark:text-orange-300',
        bgColor: 'bg-orange-50 dark:bg-orange-900/20',
        icon: '🔄',
        label: 'Initializing...',
      };
    case 'started':
      return {
        color: 'text-green-700 dark:text-green-300',
        bgColor: 'bg-green-50 dark:bg-green-900/20',
        icon: '✓',
        label: 'Active',
      };
    default:
      return {
        color: 'text-gray-700 dark:text-gray-300',
        bgColor: 'bg-gray-100 dark:bg-gray-800',
        icon: '?',
        label: 'Unknown',
      };
  }
};

export const RouteLLMStatusIndicator: React.FC<RouteLLMStatusIndicatorProps> = ({
  status,
  compact = false,
  modelPath,
  onOpenFolder,
  onDownload,
  onUnload,
  isDownloading = false,
}) => {
  const config = getStatusConfig(status.state);

  if (compact) {
    return (
      <div className="flex items-center gap-2">
        <span
          className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-sm font-medium ${config.color} ${config.bgColor}`}
        >
          <span>{config.icon}</span>
          <span>{config.label}</span>
        </span>
        {status.state === 'not_downloaded' && onDownload && !isDownloading && (
          <button
            onClick={onDownload}
            className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs font-medium bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-300 hover:bg-blue-200 dark:hover:bg-blue-900/50 transition-colors"
          >
            Download
          </button>
        )}
        {status.state === 'started' && onUnload && (
          <button
            onClick={onUnload}
            className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs font-medium bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
          >
            Unload
          </button>
        )}
        <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-purple-100 text-purple-900 dark:bg-purple-900/30 dark:text-purple-300">
          EXPERIMENTAL
        </span>
      </div>
    );
  }

  return (
    <div className={`flex items-start gap-3 p-4 rounded-lg ${config.bgColor}`}>
      <span className="text-3xl">{config.icon}</span>
      <div className="flex-1">
        <div className="flex items-center gap-2 mb-1">
          <span className={`font-semibold ${config.color}`}>{config.label}</span>
          <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-purple-100 text-purple-900 dark:bg-purple-900/30 dark:text-purple-300">
            EXPERIMENTAL
          </span>
        </div>

        {status.memory_usage_mb && (
          <div className="text-sm text-gray-600 dark:text-gray-400 mb-1">
            Memory: {(status.memory_usage_mb / 1024).toFixed(2)} GB
          </div>
        )}

        {status.last_access_secs_ago !== null && status.last_access_secs_ago < 300 && (
          <div className="text-sm text-gray-600 dark:text-gray-400 mb-1">
            Active {Math.floor(status.last_access_secs_ago / 60)}m ago
          </div>
        )}

        {modelPath && (
          <div className="mt-2 text-xs text-gray-500 dark:text-gray-400">
            <span>Model location: </span>
            {onOpenFolder ? (
              <button
                onClick={onOpenFolder}
                className="text-blue-600 dark:text-blue-400 hover:underline focus:outline-none"
              >
                {modelPath}
              </button>
            ) : (
              <code className="bg-gray-200 dark:bg-gray-700 px-1 py-0.5 rounded">
                {modelPath}
              </code>
            )}
          </div>
        )}

        {/* Action buttons */}
        {(onDownload || onUnload) && (
          <div className="mt-3 flex gap-2">
            {status.state === 'not_downloaded' && onDownload && !isDownloading && (
              <button
                onClick={onDownload}
                className="px-3 py-1.5 text-sm font-medium rounded-md bg-blue-600 text-white hover:bg-blue-700 transition-colors"
              >
                Download
              </button>
            )}
            {status.state === 'started' && onUnload && (
              <button
                onClick={onUnload}
                className="px-3 py-1.5 text-sm font-medium rounded-md bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300 hover:bg-gray-300 dark:hover:bg-gray-600 transition-colors"
              >
                Unload
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
