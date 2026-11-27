/* lib/api：超时、断线、2xx 但不是 JSON 的响应 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, connect } from './api';

const respond = (body: string, status = 200) => vi.fn(async () => new Response(body, { status }));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('请求', () => {
  it('JSON 照常返回；空响应体算 null', async () => {
    vi.stubGlobal('fetch', respond('{"ok":true}'));
    expect(await api.logout()).toEqual({ ok: true });
    vi.stubGlobal('fetch', respond(''));
    expect(await api.logout()).toBeNull();
  });

  it('2xx 但不是 JSON（例如代理返回的网页）：按错误处理，不把 null 当数据', async () => {
    vi.stubGlobal('fetch', respond('<html>login</html>'));
    await expect(api.workspace()).rejects.toMatchObject({ code: 'INTERNAL' });
  });

  it('服务端的错误码原样带回', async () => {
    vi.stubGlobal('fetch', respond('{"error":{"code":"NOT_FOUND","message":"卡片不存在"}}', 404));
    await expect(api.card('p/a', 1)).rejects.toMatchObject({ code: 'NOT_FOUND', message: '卡片不存在' });
  });

  it('批量卡片命令发到 /api/cards/batch', async () => {
    const fetch = vi.fn(async (path: string, init: RequestInit) => {
      expect(path).toBe('/api/cards/batch');
      expect(JSON.parse(String(init.body))).toEqual({ board: 'p/a', ids: [1, 2], op: 'later', on: true });
      return new Response('{"ok":[1],"failed":[{"id":2,"code":"IN_TRASH","message":"在回收站里"}],"rev":9}', { status: 200 });
    });
    vi.stubGlobal('fetch', fetch);
    expect(await api.cardBatch('p/a', [1, 2], { op: 'later', on: true })).toEqual({
      ok: [1],
      failed: [{ id: 2, code: 'IN_TRASH', message: '在回收站里' }],
      rev: 9,
    });
  });

  it('连不上、超时：NETWORK', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('Failed to fetch'))));
    await expect(api.workspace()).rejects.toMatchObject({ code: 'NETWORK' });
    // 请求一直挂着：带了 AbortSignal，超时后按 NETWORK 报错
    vi.stubGlobal(
      'fetch',
      vi.fn(
        (_: string, init: RequestInit) =>
          new Promise((_, rej) => init.signal?.addEventListener('abort', () => rej(init.signal!.reason))),
      ),
    );
    vi.spyOn(AbortSignal, 'timeout').mockImplementation(() => {
      const c = new AbortController();
      setTimeout(() => c.abort(new DOMException('timed out', 'TimeoutError')), 5);
      return c.signal;
    });
    await expect(api.cmd('p/a', { op: 'edit', id: 1, title: 'x' })).rejects.toMatchObject({ code: 'NETWORK', message: expect.stringContaining('超时') });
  });

  it('列文件夹单独用 25 秒超时：服务端最多等 board host 20 秒', async () => {
    const seen: number[] = [];
    vi.spyOn(AbortSignal, 'timeout').mockImplementation(ms => {
      seen.push(ms);
      return new AbortController().signal;
    });
    vi.stubGlobal('fetch', respond('{"dirs":[]}'));
    await api.hostLs('mbp', '~');
    expect(seen[0]).toBeGreaterThanOrEqual(20000);
  });
});

describe('推送', () => {
  class Source {
    static last: Source;
    readyState = 1;
    onopen: (() => void) | null = null;
    onmessage: ((e: { data: string }) => void) | null = null;
    onerror: (() => void) | null = null;
    constructor() {
      Source.last = this;
    }
    close() {
      this.readyState = 2;
    }
  }

  it('只吞掉 JSON 解析失败；正常推送交给 onMessage', () => {
    vi.stubGlobal('EventSource', Source);
    const onMessage = vi.fn();
    const stop = connect(onMessage, () => {});
    Source.last.onmessage!({ data: '{"rev":3,"events":[]}' });
    expect(onMessage).toHaveBeenCalledWith({ rev: 3, events: [] });
    Source.last.onmessage!({ data: '不是 JSON' });
    expect(onMessage).toHaveBeenCalledTimes(1);
    stop();
  });
});
