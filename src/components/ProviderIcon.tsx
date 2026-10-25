/**
 * Provider icon component - wrapper around ServiceIcon for backward compatibility
 */

import ServiceIcon from './ServiceIcon'

interface ProviderIconProps {
  providerId: string
  instanceName?: string
  size?: number
  className?: string
}

export default function ProviderIcon({ providerId, instanceName, size = 32, className = '' }: ProviderIconProps) {
  return <ServiceIcon resourceKey={instanceName ? `provider:${instanceName}` : undefined} service={providerId} size={size} className={className} />
}
