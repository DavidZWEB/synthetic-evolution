import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

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
function sourceRevision(command) {
  const cwd = fileURLToPath(new URL('../', import.meta.url));
  const revision = execFileSync('git', ['rev-parse', 'HEAD'], { cwd, encoding: 'utf8' }).trim();
  if (command === 'serve') return `${revision}-development`;
  const dirty = execFileSync('git', [
    'status', '--porcelain', '--untracked-files=normal', '--',
    'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'sim-core/src',
    'sim-core/Cargo.toml', 'shells/wasm', 'shells/shared', 'web', 'scripts',
  ], { cwd, encoding: 'utf8' }).trim();
  return `${revision}${dirty ? '-dirty' : ''}`;
}

export default defineConfig(({ mode, command }) => {
  const serverOptions = mode === 'transferable' ? {} : { headers: crossOriginIsolation };
  return {
    plugins: [svelte()],
    define: { __SOURCE_REVISION__: JSON.stringify(sourceRevision(command)) },
    server: serverOptions,
    preview: serverOptions,
  };
});
