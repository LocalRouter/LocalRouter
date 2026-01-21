import { useState } from 'react'
import Button from '../ui/Button'
import Select from '../ui/Select'
import Input from '../ui/Input'
import Card from '../ui/Card'

export interface StrategyRateLimit {
  limit_type: 'requests' | 'total_tokens' | 'cost'
  value: number
  time_window: 'minute' | 'hour' | 'day'
}

interface RateLimitEditorProps {
  limits: StrategyRateLimit[]
  onChange: (limits: StrategyRateLimit[]) => void
  disabled?: boolean
}

export default function RateLimitEditor({ limits, onChange, disabled = false }: RateLimitEditorProps) {
  const [showAdd, setShowAdd] = useState(false)
  const [newLimit, setNewLimit] = useState<StrategyRateLimit>({
    limit_type: 'requests',
    value: 100,
    time_window: 'hour',
  })

  const handleAdd = () => {
    onChange([...limits, { ...newLimit }])
    setShowAdd(false)
    setNewLimit({
      limit_type: 'requests',
      value: 100,
      time_window: 'hour',
    })
  }

  const handleRemove = (index: number) => {
    onChange(limits.filter((_, i) => i !== index))
  }

  const handleUpdate = (index: number, field: keyof StrategyRateLimit, value: any) => {
    const updated = [...limits]
    updated[index] = { ...updated[index], [field]: value }
    onChange(updated)
  }

  const getLimitTypeLabel = (type: string) => {
    switch (type) {
      case 'requests':
        return 'Requests'
      case 'total_tokens':
        return 'Total Tokens'
      case 'cost':
        return 'Cost (USD)'
      default:
        return type
    }
  }

  const getTimeWindowLabel = (window: string) => {
    switch (window) {
      case 'minute':
        return 'per minute'
      case 'hour':
        return 'per hour'
      case 'day':
        return 'per day'
      default:
        return window
    }
  }

  return (
    <div className="space-y-4">
      {limits.length === 0 && !showAdd && (
        <div className="text-center py-6 text-gray-500 dark:text-gray-400 text-sm">
          No rate limits configured. Click "Add Limit" to create one.
        </div>
      )}

      {limits.map((limit, index) => (
        <Card key={index}>
          <div className="flex items-center gap-4">
            <div className="flex-1 grid grid-cols-3 gap-3">
              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Type</label>
                <Select
                  value={limit.limit_type}
                  onChange={(e) => handleUpdate(index, 'limit_type', e.target.value)}
                  disabled={disabled}
                  className="w-full text-sm"
                >
                  <option value="requests">Requests</option>
                  <option value="total_tokens">Total Tokens</option>
                  <option value="cost">Cost (USD)</option>
                </Select>
              </div>

              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Limit</label>
                <Input
                  type="number"
                  value={limit.value}
                  onChange={(e) => handleUpdate(index, 'value', parseFloat(e.target.value) || 0)}
                  disabled={disabled}
                  min="0"
                  step={limit.limit_type === 'cost' ? '0.01' : '1'}
                  className="w-full text-sm"
                />
              </div>

              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Window</label>
                <Select
                  value={limit.time_window}
                  onChange={(e) => handleUpdate(index, 'time_window', e.target.value)}
                  disabled={disabled}
                  className="w-full text-sm"
                >
                  <option value="minute">Per Minute</option>
                  <option value="hour">Per Hour</option>
                  <option value="day">Per Day</option>
                </Select>
              </div>
            </div>

            <div className="flex items-end pb-1">
              <Button
                onClick={() => handleRemove(index)}
                disabled={disabled}
                variant="danger"
              >
                Remove
              </Button>
            </div>
          </div>

          <div className="mt-2 text-xs text-gray-600 dark:text-gray-400">
            <strong>
              {limit.value.toLocaleString()} {getLimitTypeLabel(limit.limit_type)}
            </strong>{' '}
            {getTimeWindowLabel(limit.time_window)}
          </div>
        </Card>
      ))}

      {showAdd && (
        <Card>
          <div className="space-y-3">
            <h4 className="font-medium text-gray-900 dark:text-gray-100 text-sm">Add New Rate Limit</h4>

            <div className="grid grid-cols-3 gap-3">
              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Type</label>
                <Select
                  value={newLimit.limit_type}
                  onChange={(e) => setNewLimit({ ...newLimit, limit_type: e.target.value as any })}
                  className="w-full text-sm"
                >
                  <option value="requests">Requests</option>
                  <option value="total_tokens">Total Tokens</option>
                  <option value="cost">Cost (USD)</option>
                </Select>
              </div>

              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Limit</label>
                <Input
                  type="number"
                  value={newLimit.value}
                  onChange={(e) => setNewLimit({ ...newLimit, value: parseFloat(e.target.value) || 0 })}
                  min="0"
                  step={newLimit.limit_type === 'cost' ? '0.01' : '1'}
                  className="w-full text-sm"
                />
              </div>

              <div>
                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Window</label>
                <Select
                  value={newLimit.time_window}
                  onChange={(e) => setNewLimit({ ...newLimit, time_window: e.target.value as any })}
                  className="w-full text-sm"
                >
                  <option value="minute">Per Minute</option>
                  <option value="hour">Per Hour</option>
                  <option value="day">Per Day</option>
                </Select>
              </div>
            </div>

            <div className="flex gap-2">
              <Button onClick={handleAdd}>
                Add Limit
              </Button>
              <Button onClick={() => setShowAdd(false)} variant="secondary">
                Cancel
              </Button>
            </div>
          </div>
        </Card>
      )}

      {!showAdd && !disabled && (
        <Button onClick={() => setShowAdd(true)} variant="secondary">
          + Add Rate Limit
        </Button>
      )}
    </div>
  )
}
