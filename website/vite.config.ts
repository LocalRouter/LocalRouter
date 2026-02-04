import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'
import type { Plugin } from 'vite'

// Custom plugin to resolve @/ imports based on the importing file's location
function resolveAppAlias(): Plugin {
  const mainAppSrc = path.resolve(__dirname, "../src")
  const websiteSrc = path.resolve(__dirname, "./src")

  return {
    name: 'resolve-app-alias',
    enforce: 'pre',
    resolveId(source, importer) {
      // Only handle @/ imports
      if (!source.startsWith('@/')) {
        return null
      }

      // Determine which src folder to use based on where the import is from
      if (importer && importer.includes('/localrouterai/src/') && !importer.includes('/localrouterai/website/')) {
        // Import from main app - resolve to main app's src
        const resolved = source.replace('@/', mainAppSrc + '/')
        return this.resolve(resolved, importer, { skipSelf: true })
      }

      // Import from website - resolve to website's src
      const resolved = source.replace('@/', websiteSrc + '/')
      return this.resolve(resolved, importer, { skipSelf: true })
    }
  }
}

// Plugin to transform import.meta.env.DEV to false for main app code
// This hides debug menus in the demo
function forceProductionForApp(): Plugin {
  const mainAppSrc = path.resolve(__dirname, "../src")

  return {
    name: 'force-production-for-app',
    enforce: 'pre',
    transform(code, id) {
      // Only transform main app files
      if (!id.startsWith(mainAppSrc)) {
        return null
      }

      let transformed = code

      // Replace import.meta.env.DEV with false
      if (transformed.includes('import.meta.env.DEV')) {
        transformed = transformed.replace(/import\.meta\.env\.DEV/g, 'false')
      }

      // Replace require() calls with empty object (require doesn't exist in browser)
      // This handles dynamic imports like: const { X } = require("...")
      if (transformed.includes('require(')) {
        transformed = transformed.replace(/require\([^)]+\)/g, '({ MCP_SERVER_TEMPLATES: [] })')
      }

      if (transformed !== code) {
        return {
          code: transformed,
          map: null
        }
      }

      return null
    }
  }
}

export default defineConfig({
  plugins: [
    resolveAppAlias(),
    forceProductionForApp(),
    react(),
  ],
  base: '/',
  resolve: {
    alias: {
      "@app": path.resolve(__dirname, "../src"),       // Main Tauri app src
      // Stub Tauri plugins for demo mode
      "@tauri-apps/api/core": path.resolve(__dirname, "./src/stubs/tauri-api-core.ts"),
      "@tauri-apps/api/event": path.resolve(__dirname, "./src/stubs/tauri-api-event.ts"),
      "@tauri-apps/api/mocks": path.resolve(__dirname, "./src/stubs/tauri-api-mocks.ts"),
      "@tauri-apps/api/webviewWindow": path.resolve(__dirname, "./src/stubs/tauri-api-webviewWindow.ts"),
      "@tauri-apps/plugin-dialog": path.resolve(__dirname, "./src/stubs/tauri-plugin-dialog.ts"),
      "@tauri-apps/plugin-shell": path.resolve(__dirname, "./src/stubs/tauri-plugin-shell.ts"),
      "@tauri-apps/plugin-updater": path.resolve(__dirname, "./src/stubs/tauri-plugin-updater.ts"),
      "@tauri-apps/plugin-process": path.resolve(__dirname, "./src/stubs/tauri-plugin-process.ts"),
      // Stub OpenAI client for demo mode (Try it out feature)
      "openai": path.resolve(__dirname, "./src/stubs/openai.ts"),
    },
  },
  optimizeDeps: {
    include: ['@tauri-apps/api'],
  },
})
