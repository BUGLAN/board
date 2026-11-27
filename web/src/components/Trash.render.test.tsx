import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { TrashCard } from '../lib/types';
import { TrashRow } from './Trash';

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
});

describe('回收站的描述首行', () => {
  it('只有小标题时显示小标题', () => {
    const t: TrashCard = {
      id: 42,
      title: '登录页按钮圆角',
      body: '### 设置页也一样',
      column: 'todo',
      deletedAt: 0,
      deletedBy: { kind: 'human', name: '' },
    } as TrashCard;
    act(() =>
      root.render(
        <TrashRow t={t} now={1000} statuses={[]} busy={false} checked={false} onCheck={() => {}} onRestore={() => {}} onPurge={() => {}} />,
      ),
    );
    expect(host.querySelector('.ib-t > span')?.textContent).toBe('设置页也一样');
  });
});
