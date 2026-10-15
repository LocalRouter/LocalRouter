import { test, expect } from '@playwright/test'
import { generateVocalCordApiKey } from '../../../src/utils/vocalcord-api-key'

test('generated VocalCord keys use the documented vc_ + 43 base64url format', () => {
  const keys = new Set<string>()
  for (let i = 0; i < 200; i++) {
    const key = generateVocalCordApiKey()
    expect(key).toMatch(/^vc_[A-Za-z0-9_-]{43}$/)
    keys.add(key)
  }
  expect(keys.size).toBe(200)
})
