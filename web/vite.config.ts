/// <reference types="vitest/config" />
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
    // 设置页内嵌了仓库根目录下的 .agents/skills/board-skills/SKILL.md
    fs: { allow: ['..'] },
    // /auth：GitHub 登录与回调
    proxy: { '/api': { target: api, changeOrigin: true }, '/auth': { target: api, changeOrigin: true } },
  },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    chunkSizeWarningLimit: 1200,
  },
  // 单元测试（npm test 会跑）：src 下的 *.test.ts；DOM 用 jsdom（happy-dom 下 DOMPurify 清理不干净，Markdown 测试会误判）。不访问网络（见 test/setup.ts）
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['test/setup.ts'],
  },
});
