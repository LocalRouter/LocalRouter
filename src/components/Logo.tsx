interface LogoProps {
  className?: string
}

export function Logo({ className = "h-8 w-8" }: LogoProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 100 100"
      fill="none"
      className={className}
    >
      {/* Top-left circle (hollow) */}
      <circle cx="20" cy="20" r="12" stroke="currentColor" strokeWidth="10" fill="none"/>

      {/* Bottom-right circle (hollow) */}
      <circle cx="80" cy="80" r="12" stroke="currentColor" strokeWidth="10" fill="none"/>

      {/* Smooth S-curve routing line */}
      <path
        d="M 32 22 C 75 15, 90 40, 50 50 C 10 60, 25 85, 68 78"
        stroke="currentColor"
        strokeWidth="10"
        strokeLinecap="round"
        fill="none"
      />
    </svg>
  )
}
