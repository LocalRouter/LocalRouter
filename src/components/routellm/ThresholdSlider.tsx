/**
 * Threshold Slider Component
 * Allows users to adjust the routing threshold with visual feedback
 */

import React, { useEffect } from 'react';
import { ThresholdProfile } from './types';

interface ThresholdSliderProps {
  value: number;
  onChange: (value: number) => void;
  onEstimateUpdate?: (split: { weak: number; strong: number }) => void;
}

const getProfile = (threshold: number): ThresholdProfile => {
  if (threshold >= 0.6) {
    return {
      name: 'Cost Optimized',
      weak: 70,
      strong: 30,
      savings: '60%',
      quality: '85%',
    };
  }
  if (threshold >= 0.4) {
    return {
      name: 'Balanced',
      weak: 50,
      strong: 50,
      savings: '47%',
      quality: '90%',
    };
  }
  if (threshold >= 0.2) {
    return {
      name: 'Quality Prioritized',
      weak: 25,
      strong: 75,
      savings: '24%',
      quality: '95%',
    };
  }
  return {
    name: 'Maximum Quality',
    weak: 10,
    strong: 90,
    savings: '10%',
    quality: '98%',
  };
};

export const ThresholdSlider: React.FC<ThresholdSliderProps> = ({
  value,
  onChange,
  onEstimateUpdate,
}) => {
  const profile = getProfile(value);

  useEffect(() => {
    onEstimateUpdate?.({ weak: profile.weak, strong: profile.strong });
  }, [value, profile.weak, profile.strong, onEstimateUpdate]);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
          Routing Threshold
        </label>
        <span className="inline-flex items-center px-2.5 py-1 rounded-md text-xs font-medium bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-300">
          {profile.name}
        </span>
      </div>

      <div className="relative">
        <input
          type="range"
          min="0"
          max="1"
          step="0.05"
          value={value}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          className="w-full h-2 bg-gray-200 rounded-lg appearance-none cursor-pointer dark:bg-gray-700 accent-blue-600"
        />
        <div className="flex justify-between mt-1 text-xs text-gray-500 dark:text-gray-400">
          <span>More Cheap</span>
          <span className="font-mono font-semibold text-gray-700 dark:text-gray-300">
            {value.toFixed(2)}
          </span>
          <span>More Quality</span>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="p-2 bg-gray-50 dark:bg-gray-800 rounded border border-gray-200 dark:border-gray-700">
          <div className="font-semibold text-gray-900 dark:text-gray-100">
            {profile.weak}% / {profile.strong}%
          </div>
          <div className="text-gray-600 dark:text-gray-400">Weak / Strong Split</div>
        </div>
        <div className="p-2 bg-gray-50 dark:bg-gray-800 rounded border border-gray-200 dark:border-gray-700">
          <div className="font-semibold text-green-600 dark:text-green-400">
            {profile.savings} savings
          </div>
          <div className="text-gray-600 dark:text-gray-400">{profile.quality} quality</div>
        </div>
      </div>

      <div className="p-2 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-700 rounded text-xs text-yellow-800 dark:text-yellow-200">
        <strong>Note:</strong> Estimates based on RouteLLM research paper. Actual results may
        vary depending on your workload and model selection.
      </div>
    </div>
  );
};
