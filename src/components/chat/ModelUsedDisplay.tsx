import React from 'react';
import { ModelUsedDisplayProps } from './types';

export const ModelUsedDisplay: React.FC<ModelUsedDisplayProps> = ({
  requestedModel,
  actualModel,
  contextType,
}) => {
  if (!actualModel) {
    return null;
  }

  const modelChanged = requestedModel !== actualModel;

  return (
    <div className="mt-3 px-3 py-2 rounded-lg bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-gray-600 dark:text-gray-400">
            Using model:
          </span>
          <span className="text-xs font-mono text-gray-900 dark:text-gray-100">
            {actualModel}
          </span>
        </div>
        {modelChanged && contextType === 'api_key' && (
          <div className="flex items-center gap-1 px-2 py-1 rounded bg-yellow-100 dark:bg-yellow-900/30 border border-yellow-300 dark:border-yellow-700">
            <svg
              className="w-3 h-3 text-yellow-600 dark:text-yellow-400"
              fill="currentColor"
              viewBox="0 0 20 20"
            >
              <path
                fillRule="evenodd"
                d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z"
                clipRule="evenodd"
              />
            </svg>
            <span className="text-xs font-medium text-yellow-700 dark:text-yellow-300">
              Routed
            </span>
          </div>
        )}
      </div>
      {modelChanged && contextType === 'api_key' && (
        <div className="mt-1 text-xs text-gray-500 dark:text-gray-400">
          Requested: <span className="font-mono">{requestedModel}</span>
        </div>
      )}
    </div>
  );
};
