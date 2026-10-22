import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: '.', testMatch: ['routing-profiles.spec.ts', 'decision-routing.spec.ts'], workers: 1, timeout: 45_000,
  reporter: 'list', outputDir: '../../test-results/routing-profiles',
  use: { baseURL: 'http://127.0.0.1:1433', viewport: { width: 1440, height: 1080 },
    launchOptions: process.env.PLAYWRIGHT_CHROME_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE } : {},
  },
  webServer: { command: 'npm run dev --prefix ../../website -- --host 127.0.0.1 --port 1433 --strictPort',
    url: 'http://127.0.0.1:1433/demo', reuseExistingServer: false },
})
