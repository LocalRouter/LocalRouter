/**
 * WarningBox - A reusable warning/alert component
 *
 * Displays warning messages with an icon and optional action buttons.
 * Supports different severity levels (info, warning, error, success).
 */

import { ReactNode } from 'react';

export type WarningVariant = 'info' | 'warning' | 'error' | 'success';

interface WarningBoxProps {
  /** Visual variant of the warning box */
  variant?: WarningVariant;
  /** Main title/heading of the warning */
  title: string;
  /** Warning message content (can be string or JSX) */
  message: ReactNode;
  /** Optional action message or link */
  action?: ReactNode;
  /** Optional custom icon (overrides default variant icon) */
  icon?: ReactNode;
  /** Additional CSS classes */
  className?: string;
  /** Optional click handler for the entire box */
  onClick?: () => void;
}

const VARIANT_STYLES: Record<WarningVariant, { bg: string; border: string; icon: string; iconBg: string }> = {
  info: {
    bg: 'bg-blue-50 dark:bg-blue-900/20',
    border: 'border-blue-200 dark:border-blue-700',
    icon: '💡',
    iconBg: 'bg-blue-100 dark:bg-blue-800/30',
  },
  warning: {
    bg: 'bg-yellow-50 dark:bg-yellow-900/20',
    border: 'border-yellow-200 dark:border-yellow-700',
    icon: '⚠️',
    iconBg: 'bg-yellow-100 dark:bg-yellow-800/30',
  },
  error: {
    bg: 'bg-red-50 dark:bg-red-900/20',
    border: 'border-red-200 dark:border-red-700',
    icon: '❌',
    iconBg: 'bg-red-100 dark:bg-red-800/30',
  },
  success: {
    bg: 'bg-green-50 dark:bg-green-900/20',
    border: 'border-green-200 dark:border-green-700',
    icon: '✓',
    iconBg: 'bg-green-100 dark:bg-green-800/30',
  },
};

export default function WarningBox({
  variant = 'warning',
  title,
  message,
  action,
  icon,
  className = '',
  onClick,
}: WarningBoxProps) {
  const styles = VARIANT_STYLES[variant];
  const displayIcon = icon !== undefined ? icon : styles.icon;

  return (
    <div
      className={`${styles.bg} border ${styles.border} rounded-lg p-4 ${className} ${
        onClick ? 'cursor-pointer hover:opacity-90 transition-opacity' : ''
      }`}
      onClick={onClick}
      role={onClick ? 'button' : 'alert'}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onClick();
              }
            }
          : undefined
      }
    >
      <div className="flex gap-3">
        {/* Icon */}
        <div className={`flex-shrink-0 w-8 h-8 ${styles.iconBg} rounded-full flex items-center justify-center text-lg`}>
          {displayIcon}
        </div>

        {/* Content */}
        <div className="flex-1 min-w-0">
          <h4 className="font-semibold text-gray-900 dark:text-gray-100 mb-1">{title}</h4>
          <div className="text-sm text-gray-700 dark:text-gray-300 mb-2">{message}</div>
          {action && <div className="text-sm text-gray-600 dark:text-gray-400 mt-2">{action}</div>}
        </div>
      </div>
    </div>
  );
}
