import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: '.', testMatch: 'secret-placeholders.spec.ts', workers: 1, timeout: 30_000,
  reporter: 'list', outputDir: '../../test-results/secret-placeholders',
  use: { baseURL: 'http://127.0.0.1:1437', viewport: { width: 1440, height: 1000 }, launchOptions: process.env.PLAYWRIGHT_CHROME_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE } : {} },
  webServer: { command: 'npm run dev --prefix website -- --host 127.0.0.1 --port 1437 --strictPort', cwd: '../..', url: 'http://127.0.0.1:1437', reuseExistingServer: true },
})
