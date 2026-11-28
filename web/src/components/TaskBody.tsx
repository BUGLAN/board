/* 卡片描述（Markdown）：任务列表的勾选框可以直接点（卡片详情、复盘台），翻转描述里对应的那一项并保存。
   保存走乐观更新（editCard）：当帧画上勾，失败时撤回并提示。卡片被持有、正在保存、只读时不能点；
   勾选框本身是 role=checkbox，Space / Enter 和鼠标一样切换，重画后焦点留在同一项上 */
import { useLayoutEffect, useRef } from 'react';
import { toggleTask } from '../lib/review';
import type { Card } from '../lib/types';
import { cx } from '../lib/util';
import { editCard, exec } from '../store/data';
import { toastError } from '../store/ui';
import { Markdown } from './bits';

export function TaskBody({ card: c, repo, locked, className }: { card: Card; repo?: string | null; locked?: boolean; className?: string }) {
  const root = useRef<HTMLDivElement>(null);
  const focus = useRef<number | null>(null);

  // Markdown 用 dangerouslySetInnerHTML 整体重画；键盘切换后把焦点放回同一项，免得按一下 Space 就丢焦点
  useLayoutEffect(() => {
    const n = focus.current;
    if (n == null) return;
    focus.current = null;
    root.current?.querySelectorAll<HTMLElement>('li.task > [role="checkbox"]')[n]?.focus({ preventScroll: true });
  }, [c.body]);

  const toggle = (li: HTMLElement) => {
    const all = [...(root.current?.querySelectorAll('li.task') ?? [])];
    const n = all.indexOf(li);
    if (n < 0) return;
    const body = toggleTask(c.body, n);
    if (body === c.body) return;
    exec({ op: 'edit', id: c.id, body }, editCard(c.id, { body })).catch(toastError);
  };

  return (
    <div
      ref={root}
      className={cx('task-body', !locked && 'tasks-on', className)}
      onClickCapture={e => {
        if (locked) return;
        const t = e.target as HTMLElement;
        const li = t.closest?.('li.task') as HTMLElement | null;
        // 点链接、图片照常
        if (!li || t.closest('a, img')) return;
        e.preventDefault();
        e.stopPropagation();
        toggle(li);
      }}
      onKeyDownCapture={e => {
        if (locked) return;
        if (!(e.key === ' ' || e.key === 'Enter' || e.code === 'Space')) return;
        const box = (e.target as HTMLElement).closest?.('[role="checkbox"]') as HTMLElement | null;
        const li = box?.closest('li.task') as HTMLElement | null;
        if (!li) return;
        e.preventDefault();
        e.stopPropagation();
        if (e.repeat) return;
        const n = [...(root.current?.querySelectorAll('li.task') ?? [])].indexOf(li);
        if (n >= 0) focus.current = n;
        toggle(li);
      }}
    >
      <Markdown src={c.body} repo={repo} />
    </div>
  );
}
