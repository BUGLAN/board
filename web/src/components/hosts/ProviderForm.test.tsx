/* 添加模型供应商时切换常用供应商：上一家的 key、强度、获取到的模型列表不能带到下一家（default/main#611） */
import { useState } from 'react';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { blankProvider } from '../../lib/providers';
import type { AgentMeta, Provider } from '../../lib/types';
import { ProviderForm } from './ProviderForm';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const META: AgentMeta[] = [
  { id: 'claude', name: 'Claude Code', protocols: ['anthropic'], efforts: ['low', 'medium', 'high'] },
  { id: 'codex', name: 'Codex', protocols: ['openai-responses'], efforts: ['minimal', 'low', 'medium', 'high'] },
];

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let current: Provider;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

const click = (el: Element | null | undefined) => act(() => (el as HTMLElement).click());
const tile = (name: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('.fm-tile')].find(b => b.querySelector('b')?.textContent === name);
const type = (el: HTMLInputElement, value: string) =>
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });

function Harness() {
  const [p, setP] = useState(blankProvider());
  current = p;
  return (
    <ProviderForm
      value={p}
      editing={false}
      busy={false}
      existing={[]}
      meta={META}
      onChange={p => {
        current = p;
        setP(p);
      }}
      onSave={() => {}}
      onCancel={() => {}}
    />
  );
}

describe('模型供应商表单', () => {
  it('切换常用供应商时清掉上一家的 key、强度、获取到的模型和手动输入', async () => {
    await act(async () => root.render(<Harness />));
    await click(tile('DeepSeek'));
    expect(current.name).toBe('deepseek');

    type(document.querySelector<HTMLInputElement>('.fm-key input')!, 'sk-deepseek');
    const levels = document.querySelector('[aria-label="deepseek-reasoner 支持的强度"]')!;
    await click([...levels.querySelectorAll('button')].find(b => b.textContent === 'high'));
    expect(current.efforts).toEqual({ 'deepseek-reasoner': ['high'] });
    type(document.querySelector<HTMLInputElement>('.fm-manual')!, 'deepseek-manual');

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ models: ['deepseek-extra'], from: 'https://api.deepseek.com/v1' }), { status: 200 })),
    );
    await click([...document.querySelectorAll('button')].find(b => b.textContent?.includes('获取模型')));
    await act(async () => {});
    expect([...document.querySelectorAll('.fm-picker code')].map(x => x.textContent)).toContain('deepseek-extra');

    await click(tile('OpenAI'));
    expect(current).toMatchObject({ name: 'openai', apiKey: '', models: ['gpt-5.2', 'gpt-5-mini', 'o4-mini'], efforts: {} });
    expect(document.querySelector<HTMLInputElement>('.fm-key input')!.value).toBe('');
    expect(document.querySelector<HTMLInputElement>('.fm-manual')!.value).toBe('');
    expect(document.querySelector('.fm-picker')).toBeNull();
    expect(document.querySelector('.fm-eff-lv[aria-checked="true"]')).toBeNull();
  });
});
