import { expect, test } from 'bun:test'
import { fixtureEnvGranted } from './tests'

test('fixtureEnvGranted is true only for a non-empty allow.env list', () => {
  expect(fixtureEnvGranted(undefined)).toBe(false)
  expect(fixtureEnvGranted({})).toBe(false)
  expect(fixtureEnvGranted({ security: { allow: { read: ['./'] } } })).toBe(false)
  expect(fixtureEnvGranted({ security: { allow: { env: [] } } })).toBe(false)
  expect(fixtureEnvGranted({ security: { allow: { env: ['HOME'] } } })).toBe(true)
})
