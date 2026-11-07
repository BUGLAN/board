import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// 开发时界面跑在 1420，API 代理到 board-server（默认 7420）
const api = process.env.BOARD_API ?? 'http://127.0.0.1:7420';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // 设置页内嵌了仓库根目录下的 skills/board-skills/SKILL.md
    fs: { allow: ['..'] },
    proxy: { '/api': { target: api, changeOrigin: true } },
  },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    chunkSizeWarningLimit: 1200,
  },
});
