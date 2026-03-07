/**
 * URL validation utilities for security.
 */

/**
 * Validates that a URL uses http or https protocol.
 * Prevents javascript:, data:, or other potentially dangerous URI schemes.
 */
export function isValidHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'http:' || parsed.protocol === 'https:';
  } catch {
    return false;
  }
}
