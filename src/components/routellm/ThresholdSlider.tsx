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

// Define threshold stages with their positions (as percentage 0-1)
const STAGES = [
  { position: 0.1, short: 'Quality' },
  { position: 0.3, short: 'Balanced' },
  { position: 0.5, short: 'Cost' },
] as const;

const getProfile = (threshold: number): ThresholdProfile => {
  if (threshold >= 0.4) {
    return {
      name: 'Cost Saving',
      weak: 70,
      strong: 30,
      savings: '60%',
      quality: '85%',
    };
  }
  if (threshold >= 0.2) {
    return {
      name: 'Balanced',
      weak: 50,
      strong: 50,
      savings: '47%',
      quality: '90%',
    };
  }
  return {
    name: 'Quality Optimized',
    weak: 25,
    strong: 75,
    savings: '24%',
    quality: '95%',
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
    <div className="space-y-1 mt-4">
      <div className="space-y-1">
        <div className="flex items-center justify-between">
          <label className="text-sm font-medium text-foreground">
            Selection Threshold
          </label>
          <span className="font-mono text-xs text-muted-foreground">
            {value.toFixed(2)}
          </span>
        </div>
        <p className="text-xs text-muted-foreground">
          Higher values route more requests to weak models, saving costs but potentially reducing quality.
        </p>
      </div>

      {/* Slider */}
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-full h-2 bg-muted rounded-lg appearance-none cursor-pointer accent-primary"
      />

      {/* Stage markers positioned below slider */}
      <div className="relative h-6">
        {STAGES.map((stage) => {
          const isActive = Math.abs(value - stage.position) < 0.05;
          // Position as percentage, with small offset for centering
          const leftPercent = stage.position * 100;
          return (
            <button
              key={stage.position}
              type="button"
              onClick={() => onChange(stage.position)}
              style={{ left: `${leftPercent}%` }}
              className={`absolute -translate-x-1/2 text-[10px] px-1.5 py-0.5 rounded transition-colors ${
                isActive
                  ? 'bg-primary/20 text-primary font-medium'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              {stage.short}
            </button>
          );
        })}
      </div>
    </div>
  );
};
