import { expect, test } from '@playwright/test'
import { decodeJwt } from '../../../src/utils/jwt'

const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url')
const header = part({ alg: 'HS256', typ: 'JWT' })

test('decodes base64url and UTF-8 claims without interpreting or verifying them', () => {
  const payload = { sub: '123', name: 'Mátúš 👋', scope: ['read', 'write'], exp: 0, admin: false, nested: { value: null } }
  expect(decodeJwt(`${header}.${part(payload)}.unverified`)).toEqual({ header: { alg: 'HS256', typ: 'JWT' }, payload })
})

test('accepts unsigned tokens for display without claiming verification', () => {
  expect(decodeJwt(`${part({ alg: 'none' })}.${part({})}.`)).toEqual({ header: { alg: 'none' }, payload: {} })
})

test('malformed, masked, non-object and encrypted tokens fail safely', () => {
  for (const token of [
    '', 'masked***', `${header}.${part({})}`, `${header}.${part({})}.sig.extra`,
    `${header}.***.sig`, `${header}.a.sig`, `${header}._w.sig`,
    `${header}.${Buffer.from('not json').toString('base64url')}.sig`,
    ...[null, [], 123, 'text'].map(value => `${header}.${part(value)}.sig`),
    `${part([])}.${part({})}.sig`,
  ]) expect(decodeJwt(token), token).toBeNull()
})
