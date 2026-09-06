import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Normal development and preview exercise the shared-buffer path. The explicit
// transferable mode omits these headers so browser checks can cover the fallback too.
// SharedArrayBuffer gates on crossOriginIsolated (spec §7.7).
const crossOriginIsolation = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

// `public/staticwebapp.config.json` is copied verbatim into `dist/`, which is what
// Azure deploys (`output_location`). A config at the repo root with a build
// subdirectory is silently ignored — the most common way isolation "breaks" with no
// error at all (spec §7.7).
export default defineConfig(({ mode }) => {
  const serverOptions = mode === 'transferable' ? {} : { headers: crossOriginIsolation };
  return {
    plugins: [svelte()],
    server: serverOptions,
    preview: serverOptions,
  };
});
