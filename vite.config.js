import { defineConfig } from 'vite';

export default defineConfig({
  root: 'src',
  build: {
    outDir: '../frontend/dist',
    emptyOutDir: true,
  },
  server: {
    port: 1420,
    strictPort: true,
  },
});
