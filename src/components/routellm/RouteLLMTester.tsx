/**
 * RouteLLM Tester Component
 * Allows users to test routing predictions with custom prompts
 */

import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RouteLLMTestResult } from './types';

interface RouteLLMTesterProps {
  threshold: number;
}

export const RouteLLMTester: React.FC<RouteLLMTesterProps> = ({ threshold }) => {
  const [prompt, setPrompt] = useState('');
  const [result, setResult] = useState<RouteLLMTestResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleTest = async () => {
    if (!prompt.trim()) {
      setError('Please enter a prompt');
      return;
    }

    setLoading(true);
    setError(null);
    setResult(null);

    try {
      const testResult = await invoke<RouteLLMTestResult>('routellm_test_prediction', {
        prompt,
        threshold,
      });
      setResult(testResult);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const examplePrompts = [
    'What is 2 + 2?',
    'Explain quantum entanglement in detail',
    'Write a hello world program',
    'Analyze the economic implications of climate change',
  ];

  return (
    <div className="space-y-3">
      <h4 className="font-semibold text-gray-900 dark:text-gray-100">Try It Out</h4>

      <div className="space-y-2">
        <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
          Test Prompt
        </label>
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Enter a prompt to test routing..."
          className="w-full p-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-none"
          rows={3}
        />
      </div>

      <div className="flex flex-wrap gap-2">
        <span className="text-xs text-gray-600 dark:text-gray-400">Quick examples:</span>
        {examplePrompts.map((example, idx) => (
          <button
            key={idx}
            onClick={() => setPrompt(example)}
            className="text-xs px-2 py-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
          >
            {example.substring(0, 30)}...
          </button>
        ))}
      </div>

      <button
        onClick={handleTest}
        disabled={!prompt.trim() || loading}
        className="w-full px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700 disabled:bg-gray-400 dark:disabled:bg-gray-600 disabled:cursor-not-allowed transition-colors font-medium"
      >
        {loading ? 'Testing...' : 'Test Routing'}
      </button>

      {error && (
        <div className="p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-700 rounded-lg">
          <p className="text-sm text-red-800 dark:text-red-200">{error}</p>
        </div>
      )}

      {result && (
        <div className="p-4 bg-gray-50 dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 space-y-3">
          <div className="flex items-center justify-between">
            <span className="font-semibold text-gray-900 dark:text-gray-100">
              Routing Decision:
            </span>
            <span
              className={`inline-flex items-center px-3 py-1 rounded-md text-sm font-medium ${
                result.is_strong
                  ? 'bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-300'
                  : 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-300'
              }`}
            >
              {result.is_strong ? '💪 Strong Model' : '⚡ Weak Model'}
            </span>
          </div>

          <div>
            <div className="flex items-center justify-between mb-1 text-sm">
              <span className="text-gray-700 dark:text-gray-300">Confidence:</span>
              <span className="font-mono font-semibold text-gray-900 dark:text-gray-100">
                {(result.win_rate * 100).toFixed(1)}%
              </span>
            </div>
            <div className="relative h-8 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
              <div
                className={`absolute h-full transition-all ${
                  result.is_strong
                    ? 'bg-gradient-to-r from-orange-400 to-orange-600'
                    : 'bg-gradient-to-r from-green-400 to-green-600'
                }`}
                style={{ width: `${result.win_rate * 100}%` }}
              />
              <div className="absolute inset-0 flex items-center justify-center">
                <span className="text-xs font-semibold text-gray-900 dark:text-white mix-blend-difference">
                  Win Rate: {(result.win_rate * 100).toFixed(1)}%
                </span>
              </div>
            </div>
          </div>

          <div className="flex items-center justify-between text-xs text-gray-600 dark:text-gray-400 pt-2 border-t border-gray-300 dark:border-gray-600">
            <span>Latency: {result.latency_ms}ms</span>
            <span>Threshold: {(threshold * 100).toFixed(0)}%</span>
          </div>

          <div className="p-2 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded text-xs text-blue-800 dark:text-blue-200">
            {result.is_strong ? (
              <>
                <strong>Strong model selected:</strong> This prompt appears complex or requires
                high-quality output. It will be routed to your configured strong models for best
                results.
              </>
            ) : (
              <>
                <strong>Weak model selected:</strong> This prompt appears simple and
                straightforward. It will be routed to your configured weak (cost-efficient) models
                to save costs.
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
