/**
 * Unified Threshold Selector Component
 * Combines slider, preset buttons, and optional try-it-out functionality
 */

import React, { useState, useEffect } from 'react';
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Input } from "@/components/ui/Input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/Slider";
import { ThresholdProfile } from './types';
import type { RouteLLMTestResult } from '@/types/tauri-commands';

// Threshold presets with their values and descriptions (ordered left-to-right: quality → cost)
const THRESHOLD_PRESETS = [
  { name: "Quality", value: 0.1, description: "Use weak model rarely — prioritize quality" },
  { name: "Balanced", value: 0.3, description: "Use weak model for simple requests (recommended)" },
  { name: "Cost Savings", value: 0.5, description: "Use weak model often — maximize savings" },
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

interface TestHistoryItem {
  prompt: string;
  score: number;
  isStrong: boolean;
  latencyMs: number;
  threshold: number;
}

interface ThresholdSelectorProps {
  value: number;
  onChange: (value: number) => void;
  onEstimateUpdate?: (split: { weak: number; strong: number }) => void;
  /** Enable the "Try it out" section with test input and history */
  showTryItOut?: boolean;
  /** Show compact version without description text */
  compact?: boolean;
}

export const ThresholdSelector: React.FC<ThresholdSelectorProps> = ({
  value,
  onChange,
  onEstimateUpdate,
  showTryItOut = false,
  compact = false,
}) => {
  const profile = getProfile(value);

  // Try it out state
  const [testPrompt, setTestPrompt] = useState("");
  const [testHistory, setTestHistory] = useState<TestHistoryItem[]>([]);
  const [isTesting, setIsTesting] = useState(false);

  useEffect(() => {
    onEstimateUpdate?.({ weak: profile.weak, strong: profile.strong });
  }, [value, profile.weak, profile.strong, onEstimateUpdate]);

  const currentPreset = THRESHOLD_PRESETS.find((p) => Math.abs(p.value - value) < 0.05);

  const runTest = async (prompt: string) => {
    if (!prompt.trim() || isTesting) return;

    setIsTesting(true);
    try {
      const result = await invoke<RouteLLMTestResult>("routellm_test_prediction", {
        prompt: prompt.trim(),
        threshold: value,
      });

      const historyItem: TestHistoryItem = {
        prompt: prompt.trim(),
        score: result.win_rate,
        isStrong: result.is_strong,
        latencyMs: result.latency_ms,
        threshold: value,
      };

      setTestHistory((prev) => [historyItem, ...prev].slice(0, 10));
      setTestPrompt("");
    } catch (err: any) {
      toast.error(`Test failed: ${err.toString()}`);
    } finally {
      setIsTesting(false);
    }
  };

  const handleTest = () => runTest(testPrompt);

  return (
    <div className="space-y-4">
      {/* Header with label */}
      <div className="flex items-center justify-between">
        <Label className="text-sm font-medium">Weak Model Usage</Label>
        <Badge variant="outline" className="text-xs">
          {profile.name}
        </Badge>
      </div>

      {/* Slider with labels */}
      <div className="space-y-2">
        <Slider
          min={0}
          max={1}
          step={0.01}
          value={[value]}
          onValueChange={([v]) => onChange(v)}
        />
        {!compact && (
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>Use weak model less</span>
            <span>Use weak model more</span>
          </div>
        )}
      </div>

      {/* Preset buttons */}
      <div className="flex gap-2">
        {THRESHOLD_PRESETS.map((preset) => {
          const isActive = Math.abs(preset.value - value) < 0.05;
          return (
            <Button
              key={preset.name}
              type="button"
              variant={isActive ? "default" : "outline"}
              size="sm"
              className="flex-1"
              onClick={() => onChange(preset.value)}
            >
              {preset.name}
            </Button>
          );
        })}
      </div>

      {/* Try it out section */}
      {showTryItOut && (
        <div className="space-y-4 pt-4 border-t">
          <Label className="text-sm font-medium">Try it out</Label>

          {/* Quick example buttons */}
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={isTesting}
              className="h-7 text-xs"
              onClick={() => runTest("Hello, how are you today?")}
            >
              Greeting
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={isTesting}
              className="h-7 text-xs"
              onClick={() => runTest("What color is the sky?")}
            >
              Factual
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={isTesting}
              className="h-7 text-xs"
              onClick={() => runTest("What is the capital of France?")}
            >
              Trivia
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={isTesting}
              className="h-7 text-xs"
              onClick={() => runTest("Write a Python function that implements a binary search tree with insert, delete, and search operations, including proper balancing.")}
            >
              Coding
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={isTesting}
              className="h-7 text-xs"
              onClick={() => runTest("Find the degree for the given field extension Q(sqrt(2), sqrt(3), sqrt(18)) over Q.")}
            >
              Graduate Math
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={isTesting}
              className="h-7 text-xs"
              onClick={() => runTest("Write a proof by induction that the sum of the first n positive integers equals n(n+1)/2.")}
            >
              Proof
            </Button>
          </div>

          {/* Test Input */}
          <div className="flex gap-2">
            <Input
              value={testPrompt}
              onChange={(e) => setTestPrompt(e.target.value)}
              placeholder="Type a prompt and press Enter..."
              onKeyDown={(e) => e.key === "Enter" && !isTesting && handleTest()}
            />
            <Button onClick={handleTest} disabled={isTesting || !testPrompt.trim()}>
              {isTesting ? (
                <>
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  Testing
                </>
              ) : (
                "Test"
              )}
            </Button>
          </div>

          {/* Test History */}
          {testHistory.length > 0 && (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs text-muted-foreground">Test History</span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setTestHistory([])}
                >
                  Clear
                </Button>
              </div>

              <div className="space-y-2 max-h-64 overflow-y-auto">
                {testHistory.map((item, idx) => {
                  // Re-evaluate routing based on current threshold
                  const wouldUseStrong = item.score >= value;
                  return (
                    <div
                      key={idx}
                      className="p-3 bg-muted rounded-lg space-y-2"
                    >
                      <p className="text-sm">
                        <span className="text-muted-foreground">&gt;</span> {item.prompt}
                      </p>
                      <div className="flex items-center justify-between text-xs">
                        <span className="text-muted-foreground">
                          Complexity:{" "}
                          <span className="font-mono text-primary">
                            {(item.score * 100).toFixed(0)}%
                          </span>
                        </span>
                        <Badge variant={wouldUseStrong ? "default" : "secondary"}>
                          → {wouldUseStrong ? "Strong" : "Weak"} model
                        </Badge>
                      </div>
                      {/* Progress bar with threshold indicator */}
                      <div className="relative h-2 w-full">
                        <div className="absolute inset-0 bg-secondary rounded-full overflow-hidden">
                          <div
                            className="h-full bg-primary rounded-full"
                            style={{ width: `${item.score * 100}%` }}
                          />
                        </div>
                        <div
                          className="absolute w-0.5 bg-muted-foreground rounded-full"
                          style={{ left: `${value * 100}%`, top: '-4px', bottom: '-4px' }}
                          title={`Threshold: ${(value * 100).toFixed(0)}%`}
                        />
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {item.latencyMs}ms
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

// Re-export for backward compatibility
export { getProfile, THRESHOLD_PRESETS };
