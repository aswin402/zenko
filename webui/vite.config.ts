import path from 'node:path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      { find: /^lucide-react$/, replacement: path.resolve(__dirname, './src/lib/themed-lucide.tsx') },
      { find: '@', replacement: path.resolve(__dirname, './src') },
    ],
  },
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:7878',
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
    chunkSizeWarningLimit: 700,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/echarts') || id.includes('node_modules/zrender')) {
            return 'vendor-echarts'
          }
          if (id.includes('node_modules/motion') || id.includes('node_modules/framer-motion')) {
            return 'vendor-motion'
          }
          if (
            id.includes('node_modules/react-markdown') ||
            id.includes('node_modules/remark-') ||
            id.includes('node_modules/micromark') ||
            id.includes('node_modules/mdast-') ||
            id.includes('node_modules/hast-') ||
            id.includes('node_modules/unified')
          ) {
            return 'vendor-markdown'
          }
          if (id.includes('src/lib/icon-packs')) {
            return 'zenko-icon-packs'
          }
          return undefined
        },
      },
    },
  },
})
