/// <reference types="vitest/config" />
import { readFileSync } from 'node:fs';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// 开发时界面跑在 1420，API 代理到 board-server（默认 7420）
const api = process.env.BOARD_API ?? 'http://127.0.0.1:7420';

/** 这份界面打包时的版本：和服务端 GET /api/info 的版本不一样时，开着的旧页面会提示刷新（default/main#368） */
const version = (() => {
  try {
    return (JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')) as { version?: string }).version ?? '';
  } catch {
    return '';
  }
})();

export default defineConfig({
  plugins: [react()],
  // 只用 VITE_ 前缀的注入：test / typecheck 也认这个类型
  define: { 'import.meta.env.VITE_BOARD_VERSION': JSON.stringify(version) },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // 设置页内嵌了仓库根目录下的 .agents/skills/board-skills/SKILL.md
    fs: { allow: ['..'] },
    // /auth：GitHub 登录与回调；/install.sh、/install.ps1：命令行版 board 的安装脚本（「添加机器」给出的命令用）
    proxy: {
      '/api': { target: api, changeOrigin: true },
      '/auth': { target: api, changeOrigin: true },
      '^/install\\.(sh|ps1)$': { target: api, changeOrigin: true },
    },
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
