/**
 * Provider icon component - wrapper around ServiceIcon for backward compatibility
 */

import ServiceIcon from './ServiceIcon'

interface ProviderIconProps {
  providerId: string
  size?: number
  className?: string
}

export default function ProviderIcon({ providerId, size = 32, className = '' }: ProviderIconProps) {
  return <ServiceIcon service={providerId} size={size} className={className} />
}
