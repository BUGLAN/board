/* 卡片描述（Markdown）：任务列表的勾选框可以直接点（卡片详情、复盘台），翻转描述里对应的那一项并保存。
   保存走乐观更新（editCard）：当帧画上勾，失败时撤回并提示。卡片被持有、正在保存、只读时不能点 */
import { toggleTask } from '../lib/review';
import type { Card } from '../lib/types';
import { cx } from '../lib/util';
import { editCard, exec } from '../store/data';
import { toastError } from '../store/ui';
import { Markdown } from './bits';

export function TaskBody({ card: c, repo, locked, className }: { card: Card; repo?: string | null; locked?: boolean; className?: string }) {
  return (
    <div
      className={cx('task-body', !locked && 'tasks-on', className)}
      onClickCapture={e => {
        if (locked) return;
        const t = e.target as HTMLElement;
        const li = t.closest?.('li.task') as HTMLElement | null;
        // 点链接、图片照常
        if (!li || t.closest('a, img')) return;
        const all = [...e.currentTarget.querySelectorAll('li.task')];
        const n = all.indexOf(li);
        if (n < 0) return;
        e.preventDefault();
        e.stopPropagation();
        const body = toggleTask(c.body, n);
        if (body === c.body) return;
        exec({ op: 'edit', id: c.id, body }, editCard(c.id, { body })).catch(toastError);
      }}
    >
      <Markdown src={c.body} repo={repo} />
    </div>
  );
}
