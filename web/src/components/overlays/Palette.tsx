/* 命令面板：搜索卡片、跳转白板、执行命令 */
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { copyBoardPrompt, copyQuickPrompt, newBoard, newProject, openBoard, openCard } from '../../lib/actions';
import { canNewWindow, cycleTheme, getTheme, newWindow } from '../../lib/platform';
import { quickPrompts } from '../../lib/prompt';
import { boardFlow, INBOX } from '../../lib/schema';
import type { Card } from '../../lib/types';
import { accel, cx } from '../../lib/util';
import { allBoards, loadFull, useData } from '../../store/data';
import { closePalette, manageLabels, openCreator, setSide, setTab, toast, useUI } from '../../store/ui';
import { Icon, type IconName } from '../Icon';
import { usePresence } from './presence';

interface PItem {
  id: string;
  group: string;
  title: string;
  sub?: string;
  hint?: string;
  icon?: IconName;
  card?: Card;
  /** 卡片所在状态的颜色（和工作流条一致） */
  color?: string;
  q?: string;
  run: () => void;
}

function highlight(text: string, q: string): ReactNode {
  const parts = text.split(/(`[^`]+`)/g);
  const lower = q.toLowerCase();
  return parts.map((p, i) => {
    if (p.startsWith('`') && p.endsWith('`') && p.length > 1) return <code key={i}>{p.slice(1, -1)}</code>;
    const at = lower ? p.toLowerCase().indexOf(lower) : -1;
    if (at < 0) return <span key={i}>{p}</span>;
    return (
      <span key={i}>
        {p.slice(0, at)}
        <mark>{p.slice(at, at + q.length)}</mark>
        {p.slice(at + q.length)}
      </span>
    );
  });
}

export function Palette() {
  const { open, prefill } = useUI(s => s.palette);
  const { mounted, on } = usePresence(open, 200);
  const board = useData(s => s.board);
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const [q, setQ] = useState('');
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const prevFocus = useRef<Element | null>(null);

  useEffect(() => {
    if (!open) return;
    prevFocus.current = document.activeElement;
    setQ(prefill);
    setActive(0);
    const t = setTimeout(() => input.current?.focus(), 30);
    return () => clearTimeout(t);
  }, [open, prefill]);

  // 搜卡片时取全量：终点状态默认只带最近的几张，更早的也要能搜到
  const searching = open && !!q.trim();
  useEffect(() => {
    if (searching) void loadFull();
  }, [searching]);

  const close = () => {
    closePalette();
    (prevFocus.current as HTMLElement | null)?.focus?.({ preventScroll: true });
  };

  const items = useMemo<PItem[]>(() => {
    if (!open) return [];
    const query = q.trim();
    const out: PItem[] = [];
    const f = boardFlow(board);
    const all = board ? [INBOX, ...f.list.map(s => s.key)].flatMap(col => (board.cols[col] ?? []).map(id => board.cards[id])).filter(Boolean) : [];
    let cards: Card[];
    if (!query) cards = [...all].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 6);
    else {
      const ql = query.toLowerCase().replace(/^#/, '');
      cards = all.filter(c => String(c.id) === ql || c.title.toLowerCase().includes(ql) || c.body.toLowerCase().includes(ql)).slice(0, 12);
    }
    for (const c of cards)
      out.push({
        id: `card-${c.id}`, group: query ? '卡片' : '最近更新', title: c.title, card: c, color: f.color(c.column), q: query.replace(/^#/, ''),
        sub: c.holder ? `${f.name(c.column)} · ${c.holder}` : f.name(c.column),
        run: () => openCard(c.id),
      });
    const theme = { auto: '跟随系统', light: '浅色', dark: '深色' }[getTheme()];
    const acts: PItem[] = [
      { id: 'new', group: '操作', title: '新建卡片', hint: 'N', icon: 'plus', run: () => openCreator('todo') },
      { id: 'capture', group: '操作', title: '记一条未整理', hint: 'I', icon: 'inbox', run: () => openCreator('inbox') },
      { id: 'board-prompt', group: '操作', title: '复制给 agent：处理这块白板的待办', sub: `/board-skills ${path}`, icon: 'prompt', run: () => copyBoardPrompt() },
      ...quickPrompts(path, f.claimables.map(s => s.name)).map(q => ({
        id: `quick-${q.id}`, group: '提示词', title: `复制提示词：${q.title}`, sub: q.text, icon: 'prompt' as IconName, run: () => copyQuickPrompt(q),
      })),
      // 带标签的提示词：每个可领取状态 × 每个标签各一条，只在搜索时出现，免得空面板被刷屏
      ...(query
        ? (ws?.projects.find(p => p.key === path.split('/')[0])?.labels ?? []).flatMap(l =>
            quickPrompts(path, f.claimables.map(s => s.name), l.name)
              .filter(q => q.kind === 'label')
              .map(q => ({
                id: `quick-${q.id}-${l.id}`, group: '提示词', title: `复制提示词：${q.title}`, sub: q.text, icon: 'prompt' as IconName, run: () => copyQuickPrompt(q),
              })),
          )
        : []),
      { id: 'labels', group: '操作', title: '管理标签', icon: 'tag', run: () => manageLabels(path.split('/')[0]) },
      { id: 'tab-board', group: '切换到', title: '白板', hint: accel('1'), icon: 'board', run: () => setTab('board') },
      { id: 'tab-projects', group: '切换到', title: '项目总览（画布）', hint: accel('2'), icon: 'folder', run: () => setTab('projects') },
      { id: 'tab-inbox', group: '切换到', title: '未整理', hint: accel('3'), icon: 'inbox', run: () => setTab('inbox') },
      { id: 'tab-settings', group: '切换到', title: '设置', hint: accel('4'), icon: 'sun', run: () => setTab('settings') },
      { id: 'tab-hosts', group: '切换到', title: '机器（board host）', hint: accel('5'), icon: 'host', run: () => setTab('hosts') },
      { id: 'side', group: '切换到', title: '收起 / 展开侧栏', hint: accel('B'), icon: 'monitor', run: () => setSide(!useUI.getState().side) },
      ...(canNewWindow
        ? [{ id: 'new-window', group: '切换到', title: '新建窗口', hint: accel('N', true), icon: 'monitor' as IconName, run: () => newWindow() }]
        : []),
      {
        id: 'theme', group: '外观', title: `切换主题（当前：${theme}）`, icon: 'sun',
        run: () => {
          const t = cycleTheme();
          toast({ type: 'info', title: { auto: '主题：跟随系统', light: '主题：浅色', dark: '主题：深色' }[t], duration: 1400 });
        },
      },
    ];
    for (const b of allBoards(ws).filter(b => b.path !== path))
      acts.push({ id: `board-${b.path}`, group: '切换白板', title: `${b.project.name} / ${b.name}`, sub: b.path, icon: 'board', run: () => openBoard(b.path) });
    acts.push({ id: 'board-new', group: '切换白板', title: '新建白板…', icon: 'plus', run: () => newBoard() });
    acts.push({ id: 'proj-new', group: '切换白板', title: '新建项目…', icon: 'folder', run: () => newProject() });
    out.push(...acts.filter(a => !query || a.title.toLowerCase().includes(query.toLowerCase()) || a.sub?.includes(query)));
    return out;
  }, [open, q, board, ws, path]);

  const act = Math.min(active, Math.max(0, items.length - 1));
  useEffect(() => {
    list.current?.querySelector('.p-item.active')?.scrollIntoView({ block: 'nearest' });
  }, [act]);

  const run = (i: number) => {
    const it = items[i];
    if (!it) return;
    close();
    setTimeout(() => it.run(), 60);
  };

  if (!mounted) return null;
  let group: string | null = null;
  return (
    <div className={cx('palette-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="palette" role="dialog" aria-label="命令面板">
        <div className="palette-search">
          <Icon name="search" />
          <input
            ref={input}
            value={q}
            placeholder="搜索卡片（标题、描述、#ID）或输入命令…"
            spellCheck={false}
            onChange={e => {
              setQ(e.target.value);
              setActive(0);
            }}
            onKeyDown={e => {
              if (e.key === 'ArrowDown') {
                e.preventDefault();
                setActive((act + 1) % Math.max(1, items.length));
              } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                setActive((act - 1 + items.length) % Math.max(1, items.length));
              } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault();
                run(act);
              } else if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                close();
              }
            }}
          />
          <kbd>esc</kbd>
        </div>
        <div className="palette-list" role="listbox" ref={list}>
          {items.length === 0 && <div className="p-empty">没有匹配的结果</div>}
          {items.map((it, i) => {
            const head = it.group !== group ? <div className="p-group">{it.group}</div> : null;
            group = it.group;
            return (
              <div key={it.id}>
                {head}
                <button
                  type="button"
                  className={cx('p-item', i === act && 'active')}
                  role="option"
                  aria-selected={i === act}
                  onClick={() => run(i)}
                  onPointerMove={() => i !== act && setActive(i)}
                >
                  {it.card ? (
                    <span className="p-ic col" style={{ ['--c' as string]: it.color }} aria-hidden="true">
                      <i />
                    </span>
                  ) : (
                    <span className="p-ic">
                      <Icon name={it.icon || 'sparkle'} />
                    </span>
                  )}
                  {it.card && <span className="p-id">#{it.card.id}</span>}
                  <span className="p-main">{highlight(it.title, it.q ?? '')}</span>
                  {it.sub && <span className="p-sub">{it.sub}</span>}
                  {it.hint && <kbd>{it.hint}</kbd>}
                </button>
              </div>
            );
          })}
        </div>
        <div className="palette-foot">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> 选择
          </span>
          <span>
            <kbd>↵</kbd> 打开
          </span>
          <span>
            <kbd>esc</kbd> 关闭
          </span>
        </div>
      </div>
    </div>
  );
}

/* ============================ 确认框 ============================ */
