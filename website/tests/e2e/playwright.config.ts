import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: '.',
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL: 'http://127.0.0.1:3100',
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'bun run dev --hostname 127.0.0.1 --port 3100',
    url: 'http://127.0.0.1:3100',
    reuseExistingServer: !process.env.CI,
    env: {
      NEXTAUTH_URL: 'http://127.0.0.1:3100',
      NEXTAUTH_SECRET: 'deka-sso-e2e-nextauth-secret',
      SSO_IDP_URL: 'http://127.0.0.1:3101',
      SSO_SECRET_DEKA: 'deka-sso-e2e-client-secret',
    },
  },
  projects: [
    {
      name: 'http',
    },
  ],
})
