interface BadgeProps {
  variant: 'success' | 'error' | 'warning'
  children: React.ReactNode
}

export default function Badge({ variant, children }: BadgeProps) {
  const variants = {
    success: 'bg-green-100 text-green-800',
    error: 'bg-red-100 text-red-800',
    warning: 'bg-yellow-100 text-yellow-800',
  }

  return (
    <span className={`inline-block px-3 py-1 rounded-full text-xs font-semibold uppercase ${variants[variant]}`}>
      {children}
    </span>
  )
}
