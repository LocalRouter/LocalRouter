interface ToggleProps {
  enabled: boolean
  onChange: (enabled: boolean) => void
  disabled?: boolean
  label?: string
}

export default function Toggle({ enabled, onChange, disabled = false, label }: ToggleProps) {
  return (
    <button
      type="button"
      onClick={() => !disabled && onChange(!enabled)}
      disabled={disabled}
      className={`
        relative inline-flex h-6 w-11 items-center rounded-full transition-colors
        ${enabled ? 'bg-blue-600 dark:bg-blue-500' : 'bg-gray-300 dark:bg-gray-600'}
        ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer hover:opacity-90'}
      `}
      role="switch"
      aria-checked={enabled}
      aria-label={label}
    >
      <span
        className={`
          inline-block h-4 w-4 transform rounded-full bg-white transition-transform
          ${enabled ? 'translate-x-6' : 'translate-x-1'}
        `}
      />
    </button>
  )
}
