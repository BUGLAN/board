/* 单元测试的公共设置：测试不访问网络。要用服务端的测试自己 vi.mock('../lib/api') */
import { beforeEach } from 'vitest';

const offline = () => {
  throw new Error('单元测试不能访问网络：请 mock src/lib/api');
};
globalThis.fetch = offline as typeof fetch;
(globalThis as { EventSource?: unknown }).EventSource = offline;

// jsdom 没有 matchMedia（lib/util 的 reduceMotion 等会用到）：一律不匹配
window.matchMedia ??= (query: string) =>
  ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  }) as MediaQueryList;

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});
