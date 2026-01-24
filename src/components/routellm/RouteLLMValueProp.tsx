/**
 * RouteLLM Value Proposition Component
 * Explains the benefits of intelligent routing
 */

import React from 'react';
import { ROUTELLM_REQUIREMENTS } from './types';

export const RouteLLMValueProp: React.FC = () => {
  return (
    <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg">
      <div className="flex items-start gap-2 mb-2">
        <h4 className="font-semibold text-blue-900 dark:text-blue-100">
          🎯 Intelligent Cost Optimization
        </h4>
        <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-300">
          EXPERIMENTAL
        </span>
      </div>

      <p className="text-sm text-blue-800 dark:text-blue-200 mb-3">
        RouteLLM uses machine learning to analyze each prompt and automatically route to the
        most cost-effective model while maintaining quality. Based on research from UC Berkeley.
      </p>

      <div className="grid grid-cols-3 gap-2 text-xs">
        <div className="bg-white dark:bg-gray-800 p-2 rounded border border-blue-100 dark:border-blue-800">
          <div className="font-semibold text-green-600 dark:text-green-400">30-60%</div>
          <div className="text-gray-600 dark:text-gray-400">Cost Savings</div>
        </div>
        <div className="bg-white dark:bg-gray-800 p-2 rounded border border-blue-100 dark:border-blue-800">
          <div className="font-semibold text-blue-600 dark:text-blue-400">85-95%</div>
          <div className="text-gray-600 dark:text-gray-400">Quality Retained</div>
        </div>
        <div className="bg-white dark:bg-gray-800 p-2 rounded border border-blue-100 dark:border-blue-800">
          <div className="font-semibold text-purple-600 dark:text-purple-400">{ROUTELLM_REQUIREMENTS.PER_REQUEST_MS}ms</div>
          <div className="text-gray-600 dark:text-gray-400">Routing Time</div>
        </div>
      </div>

      <div className="mt-3 pt-3 border-t border-blue-200 dark:border-blue-700">
        <p className="text-xs text-blue-700 dark:text-blue-300">
          💡 <strong>How it works:</strong> A BERT classifier analyzes prompt complexity and
          intelligently routes simple queries to fast, cheap models and complex queries to
          powerful, expensive models.
        </p>
      </div>
    </div>
  );
};
