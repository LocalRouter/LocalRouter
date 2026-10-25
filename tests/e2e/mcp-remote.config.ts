import { defineConfig } from '@playwright/test'

const port = Number(process.env.MCP_TEST_PORT || 1431)
const baseURL = `http://127.0.0.1:${port}`
export default defineConfig({
  testDir: '.', testMatch: 'mcp-remote.spec.ts', workers: 1, timeout: 30_000,
  reporter: 'list', outputDir: '../../test-results/mcp-remote',
  use: { baseURL, viewport: { width: 1440, height: 1080 },
    launchOptions: process.env.PLAYWRIGHT_CHROME_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROME_EXECUTABLE } : {},
  },
  webServer: { command: `npm run dev --prefix ../../website -- --host 127.0.0.1 --port ${port} --strictPort`,
    url: `${baseURL}/demo`, reuseExistingServer: false },
})
