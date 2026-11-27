import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { card, snapshot } from '../../test/fixtures';
import { useData } from '../store/data';
import { useUI } from '../store/ui';
import { LaterPage } from './Inbox';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  useData.setState({ path: 'p/a', board: snapshot('p/a', { inbox: [-1, 2] }), detailId: null });
  useUI.setState({ filter: '' });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = '';
});

describe('「稍后再做」的全选', () => {
  it('⌘A 不选还在新建的临时卡', () => {
    const cards = [
      card(-1, { column: 'inbox', laterAt: 2 }),
      card(2, { column: 'inbox', laterAt: 1 }),
    ];
    act(() => root.render(<LaterPage cards={cards} labels={[]} project="p" path="p/a" />));
    act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', metaKey: true, bubbles: true })));

    const rows = [...host.querySelectorAll<HTMLElement>('.ib-row')];
    expect(rows.map(row => row.querySelector('.ib-check')?.getAttribute('aria-checked'))).toEqual(['false', 'true']);
  });
});
