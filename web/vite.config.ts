import { defineConfig, loadEnv } from 'vite';

/**
 * Dev server config.
 *
 * Prod builds emit a static bundle to `dist/` — deploy to Cloudflare Pages,
 * Vercel, or serve from chartgen itself. In production, `VITE_CHARTGEN_BASE_URL`
 * is read at runtime from the bundle; no proxy is involved.
 *
 * In dev, we proxy every chartgen endpoint to the base URL so the SPA can run
 * same-origin and avoid CORS surprises during OAuth redirects.
 */
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, '', 'VITE_');
  const target = env.VITE_CHARTGEN_BASE_URL || 'http://localhost:9315';

  return {
    server: {
      port: 5173,
      strictPort: true,
      proxy: {
        '/mcp': { target, changeOrigin: true },
        '/sse': { target, changeOrigin: true, ws: false },
        '/message': { target, changeOrigin: true },
        '/authorize': { target, changeOrigin: true },
        '/token': { target, changeOrigin: true },
        '/register': { target, changeOrigin: true },
        '/oauth': { target, changeOrigin: true },
        '/.well-known': { target, changeOrigin: true },
      },
    },
    build: {
      target: 'es2022',
      sourcemap: true,
    },
  };
});
