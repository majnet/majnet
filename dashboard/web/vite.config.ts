import path from 'node:path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// Built assets are served by nginx (dashboard/nginx.conf), which also proxies
// /api/bot and /api/recon to the WG-internal APIs. `npm run dev` can proxy to a
// live backend by setting MAJNET_API (e.g. http://majksa over the tailnet).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@': path.resolve(__dirname, './src') } },
  server: process.env.MAJNET_API
    ? { proxy: { '/api': { target: process.env.MAJNET_API, changeOrigin: true } } }
    : undefined,
})
