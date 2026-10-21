import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: '.',
  testMatch: 'secret-redaction.spec.ts',
  workers: 1,
  timeout: 30_000,
  reporter: 'list',
  outputDir: '../../test-results/secret-redaction',
  use: {
    baseURL: 'http://127.0.0.1:1431',
    viewport: { width: 1280, height: 900 },
    launchOptions: process.env.PLAYWRIGHT_CHROME_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE } : {},
  },
  webServer: {
    command: 'npm run dev --prefix website -- --host 127.0.0.1 --port 1431 --strictPort',
    cwd: '../..',
    url: 'http://127.0.0.1:1431/__preview/secret-redaction',
    reuseExistingServer: true,
  },
})
