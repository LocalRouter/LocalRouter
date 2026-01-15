import { ReactNode, ButtonHTMLAttributes } from 'react'

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'danger'
  children: ReactNode
}

export default function Button({ variant = 'primary', children, className = '', ...props }: ButtonProps) {
  const variants = {
    primary: 'bg-gradient-to-r from-indigo-500 to-purple-600 text-white hover:shadow-lg transform hover:-translate-y-0.5',
    secondary: 'bg-gray-600 text-white hover:bg-gray-700',
    danger: 'bg-red-500 text-white hover:bg-red-600',
  }

  return (
    <button
      className={`px-5 py-2.5 rounded-md text-sm font-medium transition-all ${variants[variant]} disabled:opacity-50 disabled:cursor-not-allowed ${className}`}
      {...props}
    >
      {children}
    </button>
  )
}
