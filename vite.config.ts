import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Native targets and model/benchmark assets are not frontend source. Watching
      // them can stall the development server while large native builds run.
      ignored: [
        '**/target/**',
        '**/.cache/**',
        '**/test-output/**',
        '**/assets/**',
        '**/release/**',
        '**/benchmarks/**',
        '**/src-tauri/**',
        '**/crates/**',
      ],
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: { target: 'es2022' },
});
