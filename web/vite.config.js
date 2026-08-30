import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// COOP/COEP locally as well as in production. SharedArrayBuffer gates on
// crossOriginIsolated, so without these the dev server silently exercises only the
// transferable-buffer transport and the SAB path goes untested (spec §7.7).
const crossOriginIsolation = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

// `public/staticwebapp.config.json` is copied verbatim into `dist/`, which is what
// Azure deploys (`output_location`). A config at the repo root with a build
// subdirectory is silently ignored — the most common way isolation "breaks" with no
// error at all (spec §7.7).
export default defineConfig({
  plugins: [svelte()],
  server: { headers: crossOriginIsolation },
  preview: { headers: crossOriginIsolation },
});
