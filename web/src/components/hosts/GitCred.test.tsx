/* Git 凭据卡片（default/main#430）：「用于」显示完整主机，并说明该主机下所有仓库都生效 */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useData } from '../../store/data';
import { GitCredSection } from './GitCred';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  useData.setState({ ws: null });
});

describe('Git 凭据卡片', () => {
  it('「用于」完整显示主机，不编造仓库路径，并说明该主机下所有仓库都生效', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(
          JSON.stringify({
            cred: { host: 'github.com', username: 'BUGLAN', tail: 'DnaP', commitName: 'BUGLAN', commitEmail: 'me@example.com' },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      ),
    );

    await act(async () => root.render(<GitCredSection hosts={[]} />));
    await act(async () => {});

    const line = [...host.querySelectorAll<HTMLElement>('.pv-line')].find(el => el.querySelector('.pv-k')?.textContent === '用于');
    expect(line?.querySelector('code')?.textContent).toBe('https://github.com');
    expect(line?.querySelector('.gc-scope')?.textContent).toBe('（该主机下所有仓库）');
    expect(line?.textContent).toBe('用于https://github.com（该主机下所有仓库）');
    expect(line?.textContent).not.toContain('…');
  });
});
