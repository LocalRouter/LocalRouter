/**
 * A new VocalCord API key: `vc_` + 32 random bytes, base64url without padding.
 * VocalCord provisions the account on the first authenticated call, so a
 * locally generated key needs no sign-up request.
 */
export function generateVocalCordApiKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  const base64 = btoa(String.fromCharCode(...bytes))
  return 'vc_' + base64.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}
