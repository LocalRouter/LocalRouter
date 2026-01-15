import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],

  // Vite options tailored for Tauri development
  clearScreen: false,

  server: {
    port: 1420,
    strictPort: false, // Allow automatic port selection if 1420 is in use
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
})
