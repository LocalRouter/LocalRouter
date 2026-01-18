import { useState } from 'react'
import Button from './Button'
import Input from './Input'

interface KeyValuePair {
  key: string
  value: string
}

interface KeyValueInputProps {
  value: Record<string, string>
  onChange: (value: Record<string, string>) => void
  keyPlaceholder?: string
  valuePlaceholder?: string
  secretValue?: boolean
}

export default function KeyValueInput({
  value,
  onChange,
  keyPlaceholder = 'Key',
  valuePlaceholder = 'Value',
  secretValue = false,
}: KeyValueInputProps) {
  const pairs = Object.entries(value).map(([key, val]) => ({ key, value: val }))
  const [localPairs, setLocalPairs] = useState<KeyValuePair[]>(
    pairs.length > 0 ? pairs : [{ key: '', value: '' }]
  )

  const updatePairs = (newPairs: KeyValuePair[]) => {
    setLocalPairs(newPairs)
    // Filter out empty pairs and convert to object
    const obj: Record<string, string> = {}
    newPairs.forEach(pair => {
      if (pair.key.trim()) {
        obj[pair.key.trim()] = pair.value
      }
    })
    onChange(obj)
  }

  const handlePairChange = (index: number, field: 'key' | 'value', newValue: string) => {
    const newPairs = [...localPairs]
    newPairs[index][field] = newValue
    updatePairs(newPairs)
  }

  const addPair = () => {
    updatePairs([...localPairs, { key: '', value: '' }])
  }

  const removePair = (index: number) => {
    const newPairs = localPairs.filter((_, i) => i !== index)
    if (newPairs.length === 0) {
      updatePairs([{ key: '', value: '' }])
    } else {
      updatePairs(newPairs)
    }
  }

  return (
    <div className="space-y-2">
      {localPairs.map((pair, index) => (
        <div key={index} className="flex gap-2 items-center">
          <div className="flex-1">
            <Input
              value={pair.key}
              onChange={(e) => handlePairChange(index, 'key', e.target.value)}
              placeholder={keyPlaceholder}
            />
          </div>
          <div className="flex-1">
            <Input
              type={secretValue ? 'password' : 'text'}
              value={pair.value}
              onChange={(e) => handlePairChange(index, 'value', e.target.value)}
              placeholder={valuePlaceholder}
            />
          </div>
          <Button
            type="button"
            variant="secondary"
            onClick={() => removePair(index)}
            className="flex-shrink-0 px-3 py-1.5 text-xs"
          >
            Remove
          </Button>
        </div>
      ))}
      <Button
        type="button"
        variant="secondary"
        onClick={addPair}
        className="px-3 py-1.5 text-xs"
      >
        Add {keyPlaceholder}
      </Button>
    </div>
  )
}
