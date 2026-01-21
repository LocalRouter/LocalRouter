import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { RouteLLMTestResult } from '../routellm/types'

interface TestHistoryItem {
  prompt: string
  score: number
  isStrong: boolean
  latencyMs: number
  threshold: number
}

const THRESHOLD_PRESETS = [
  { name: 'Cost Saving', value: 0.5, description: 'Maximize cost savings (more weak model usage)' },
  { name: 'Balanced', value: 0.3, description: 'Default balanced approach (recommended)' },
  { name: 'Quality Optimized', value: 0.1, description: 'Prioritize quality (more strong model usage)' },
]

export default function ThresholdTester() {
  const [prompt, setPrompt] = useState('')
  const [threshold, setThreshold] = useState(0.3)
  const [history, setHistory] = useState<TestHistoryItem[]>([])
  const [isLoading, setIsLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const handleTest = async () => {
    if (!prompt.trim()) return

    setIsLoading(true)
    setError(null)

    try {
      const result = await invoke<RouteLLMTestResult>('routellm_test_prediction', {
        prompt: prompt.trim(),
        threshold,
      })

      const historyItem: TestHistoryItem = {
        prompt: prompt.trim(),
        score: result.win_rate,
        isStrong: result.is_strong,
        latencyMs: result.latency_ms,
        threshold,
      }

      setHistory((prev) => [historyItem, ...prev].slice(0, 10)) // Keep last 10 items
      setPrompt('')
    } catch (err: any) {
      setError(err.toString())
    } finally {
      setIsLoading(false)
    }
  }

  const handleKeyPress = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' && !isLoading) {
      handleTest()
    }
  }

  const renderScoreBar = (score: number) => {
    const percentage = Math.round(score * 100)

    return (
      <div className="font-mono text-xs text-gray-700 dark:text-gray-300 flex items-center gap-2">
        <span className="text-gray-400 dark:text-gray-500">0.0</span>
        <div className="flex-1 bg-gray-200 dark:bg-gray-700 rounded h-6 flex items-center overflow-hidden">
          <div
            className="h-full bg-gradient-to-r from-green-600 to-orange-600 transition-all duration-300"
            style={{ width: `${percentage}%` }}
          />
        </div>
        <span className="text-gray-400 dark:text-gray-500">1.0</span>
      </div>
    )
  }

  const getInterpretation = (score: number): string => {
    if (score < 0.2) return 'definitely weak'
    if (score < 0.4) return 'probably weak'
    if (score < 0.6) return 'borderline'
    if (score < 0.8) return 'probably strong'
    return 'definitely strong'
  }

  const getSelectedPreset = () => {
    return THRESHOLD_PRESETS.find((p) => Math.abs(p.value - threshold) < 0.01)
  }

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg p-6 space-y-6">
      <div>
        <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-1">Threshold Testing</h3>
        <p className="text-sm text-gray-600 dark:text-gray-400">
          Test prompts to see routing decisions and confidence scores
        </p>
      </div>

      {/* Error message */}
      {error && (
        <div className="p-3 bg-red-100 dark:bg-red-900/30 border border-red-300 dark:border-red-700 rounded text-sm text-red-800 dark:text-red-300">
          {error}
        </div>
      )}

      {/* Threshold Slider */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <label className="text-sm font-medium text-gray-700 dark:text-gray-200">Threshold</label>
          <span className="text-sm font-mono text-blue-600 dark:text-blue-400">{threshold.toFixed(2)}</span>
        </div>

        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={threshold}
          onChange={(e) => setThreshold(parseFloat(e.target.value))}
          className="w-full h-2 bg-gray-200 dark:bg-gray-700 rounded-lg appearance-none cursor-pointer accent-blue-500"
        />

        {/* Threshold Presets */}
        <div className="flex gap-2">
          {THRESHOLD_PRESETS.map((preset) => (
            <button
              key={preset.name}
              onClick={() => setThreshold(preset.value)}
              className={`flex-1 px-3 py-2 rounded text-xs font-medium transition-colors ${
                Math.abs(preset.value - threshold) < 0.01
                  ? 'bg-blue-600 text-white'
                  : 'bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'
              }`}
              title={preset.description}
            >
              {preset.name}
            </button>
          ))}
        </div>

        {getSelectedPreset() && (
          <p className="text-xs text-gray-500 dark:text-gray-400 italic">{getSelectedPreset()?.description}</p>
        )}
      </div>

      {/* Input Box */}
      <div>
        <label className="text-sm font-medium text-gray-700 dark:text-gray-200 mb-2 block">Test Prompt</label>
        <div className="flex gap-2">
          <input
            type="text"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyPress={handleKeyPress}
            placeholder="Type a prompt and press Enter..."
            disabled={isLoading}
            className="flex-1 px-4 py-2 bg-white dark:bg-gray-900 border border-gray-300 dark:border-gray-700 rounded text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
          />
          <button
            onClick={handleTest}
            disabled={isLoading || !prompt.trim()}
            className="px-4 py-2 bg-blue-600 text-white rounded font-medium hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {isLoading ? 'Testing...' : 'Test'}
          </button>
        </div>
      </div>

      {/* History */}
      {history.length > 0 && (
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-medium text-gray-700 dark:text-gray-200">Test History</h4>
            <button
              onClick={() => setHistory([])}
              className="text-xs text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 underline"
            >
              Clear
            </button>
          </div>

          <div className="space-y-3 max-h-96 overflow-y-auto">
            {history.map((item, idx) => (
              <div key={idx} className="p-4 bg-gray-50 dark:bg-gray-900/50 border border-gray-200 dark:border-gray-700 rounded space-y-2">
                {/* Prompt */}
                <div className="text-sm text-gray-700 dark:text-gray-300">
                  <span className="text-gray-400 dark:text-gray-500">&gt;</span> {item.prompt}
                </div>

                {/* Score and Bar */}
                <div className="space-y-1">
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-gray-600 dark:text-gray-400">
                      Score: <span className="font-mono text-blue-600 dark:text-blue-400">{item.score.toFixed(3)}</span>
                    </span>
                    <span className={`font-medium ${item.isStrong ? 'text-orange-600 dark:text-orange-400' : 'text-green-600 dark:text-green-400'}`}>
                      → {item.isStrong ? 'STRONG' : 'weak'} model
                    </span>
                  </div>
                  {renderScoreBar(item.score)}
                  <div className="text-xs text-gray-500 dark:text-gray-500 italic">({getInterpretation(item.score)})</div>
                </div>

                {/* Metadata */}
                <div className="flex items-center gap-4 text-xs text-gray-500 dark:text-gray-500">
                  <span>Threshold: {item.threshold.toFixed(2)}</span>
                  <span>Latency: {item.latencyMs}ms</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
