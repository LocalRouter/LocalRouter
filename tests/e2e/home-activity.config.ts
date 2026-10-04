import { defineConfig } from '@playwright/test'

// Frontend integration tests against the website's local Tauri demo, no Rust server.
export default defineConfig({
  testDir: '.',
  testMatch: 'home-activity.spec.ts',
  testIgnore: '**/unit/**',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  reporter: 'list',
  outputDir: '../../test-results/home-activity',
  use: {
    baseURL: 'http://127.0.0.1:1431',
    viewport: { width: 1440, height: 1080 },
    launchOptions: process.env.PLAYWRIGHT_CHROME_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE }
      : {},
  },
  webServer: {
    command:
      'npm run dev --prefix website -- --host 127.0.0.1 --port 1431 --strictPort',
    cwd: '../..',
    url: 'http://127.0.0.1:1431/demo',
    reuseExistingServer: true,
  },
})
