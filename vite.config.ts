import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development
  clearScreen: false,

  server: {
    port: 1420,
    strictPort: false, // Allow automatic port selection if 1420 is in use
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },

  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules')) {
            // React core
            if (id.includes('/react-dom/') || id.includes('/react/')) {
              return 'vendor-react'
            }
            // Radix UI components
            if (id.includes('@radix-ui/')) {
              return 'vendor-radix'
            }
            // Charts library
            if (id.includes('/recharts/') || id.includes('/d3-')) {
              return 'vendor-charts'
            }
            // Flow diagram library
            if (id.includes('/reactflow/') || id.includes('/@reactflow/') || id.includes('/dagre/')) {
              return 'vendor-flow'
            }
            // OpenAPI documentation
            if (id.includes('/rapidoc/')) {
              return 'vendor-rapidoc'
            }
            // MCP SDK and AI
            if (id.includes('@modelcontextprotocol/') || id.includes('/openai/') || id.includes('/ai/')) {
              return 'vendor-ai'
            }
            // Icons
            if (id.includes('/lucide-react/') || id.includes('@heroicons/')) {
              return 'vendor-icons'
            }
            // Tauri
            if (id.includes('@tauri-apps/')) {
              return 'vendor-tauri'
            }
            // Markdown
            if (id.includes('/react-markdown/') || id.includes('/remark-') || id.includes('/mdast-') || id.includes('/micromark')) {
              return 'vendor-markdown'
            }
          }
        },
      },
    },
  },
})
