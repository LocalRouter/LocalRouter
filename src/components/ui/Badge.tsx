interface BadgeProps {
  variant: 'success' | 'error' | 'warning' | 'info' | 'secondary'
  children: React.ReactNode
}

export default function Badge({ variant, children }: BadgeProps) {
  const variants = {
    success: 'bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200',
    error: 'bg-red-100 dark:bg-red-900 text-red-800 dark:text-red-200',
    warning: 'bg-yellow-100 dark:bg-yellow-900 text-yellow-800 dark:text-yellow-200',
    info: 'bg-blue-100 dark:bg-blue-900 text-blue-800 dark:text-blue-200',
    secondary: 'bg-gray-100 dark:bg-gray-700 text-gray-800 dark:text-gray-200',
  }

  return (
    <span className={`inline-block px-3 py-1 rounded-full text-xs font-semibold uppercase ${variants[variant]}`}>
      {children}
    </span>
  )
}
