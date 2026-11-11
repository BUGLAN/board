/* 右键 / ⋯ 下拉菜单：卡片、状态、项目、白板菜单，以及「移动到」「提示词」子菜单、选择框的选项 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import {
  copyBoardEnv, copyBoardPrompt, copyQuickPrompt, deleteBoard, deleteProject, editBoard, editProject, newBoard, openBoard, setProjectColor,
} from '../../lib/actions';
import { moveTo, vetoFor } from '../../lib/drag';
import { quickPrompts } from '../../lib/prompt';
import { boardFlow, INBOX, INBOX_NAME } from '../../lib/schema';
import { cx } from '../../lib/util';
import { allBoards, boardMeta, colCount, useData } from '../../store/data';
import { closeCtx, manageLabels, useUI } from '../../store/ui';
import { ColorRow, LabelChip, ProjectAvatar } from '../bits';
import { Icon } from '../Icon';

export function CtxMenu() {
  const req = useUI(s => s.ctx);
  const ws = useData(s => s.ws);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!req || !ref.current) return setPos(null);
    const m = ref.current;
    const w = m.offsetWidth;
    const h = m.offsetHeight;
    const r = req.anchor ?? ({ left: req.x, right: req.x, top: req.y, bottom: req.y } as DOMRect);
    const select = req.kind === 'select';
    setPos({
      left: Math.max(8, Math.min(req.anchor && !select ? r.right - w : r.left, innerWidth - w - 8)),
      top: r.bottom + h + 8 > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6,
    });
    requestAnimationFrame(() => {
      // 选择框：焦点落在当前选中的那一项上，并滚到看得见
      const cur = select ? m.querySelector<HTMLElement>('button[data-do].on:not([disabled])') : null;
      cur?.scrollIntoView({ block: 'nearest' });
      (cur ?? m.querySelector<HTMLElement>('button[data-do]:not([disabled])'))?.focus({ preventScroll: true });
    });
  }, [req]);

  useEffect(() => {
    if (!req) return;
    const down = (e: PointerEvent) => !(e.target as HTMLElement).closest('.ctx, .prompts-btn, .col-sort-btn, .d-move, .sel') && closeCtx();
    document.addEventListener('pointerdown', down);
    window.addEventListener('resize', closeCtx);
    return () => {
      document.removeEventListener('pointerdown', down);
      window.removeEventListener('resize', closeCtx);
    };
  }, [req]);

  if (!req) return null;
  const go = (fn: () => void) => () => {
    closeCtx();
    fn();
  };
  let body: ReactNode = null;
  if (req.kind === 'project') {
    const p = ws?.projects.find(x => x.key === req.id);
    if (!p) return null;
    body = (
      <>
        <div className="ctx-title">
          <ProjectAvatar name={p.name} color={p.color} className="sm" />
          <b>{p.name}</b>
        </div>
        <button type="button" data-do="new-board" onClick={go(() => newBoard(p.key))}>
          <Icon name="plus" />
          新建白板
        </button>
        <button type="button" data-do="edit-project" onClick={go(() => editProject(p.key))}>
          <Icon name="edit" />
          编辑项目
        </button>
        <button type="button" data-do="labels" onClick={go(() => manageLabels(p.key))}>
          <Icon name="tag" />
          管理标签
        </button>
        <ColorRow className="ctx-swatches" value={p.color} onPick={c => setProjectColor(p.key, c)} />
        <div className="ctx-sep" />
        <button type="button" data-do="delete-project" className="danger" onClick={go(() => deleteProject(p.key))}>
          <Icon name="trash" />
          删除项目
        </button>
      </>
    );
  } else if (req.kind === 'prompts') {
    body = <PromptsMenu path={req.id} go={go} />;
  } else if (req.kind === 'move') {
    body = <MoveMenu id={Number(req.id)} go={go} />;
  } else if (req.kind === 'select') {
    body = (req.items ?? []).map(o => (
      <button key={o.value} type="button" role="option" aria-selected={o.value === req.value} data-do={`pick-${o.value}`} className={cx(o.value === req.value && 'on')} disabled={o.disabled} onClick={go(() => req.pick?.(o.value))}>
        <span className="sel-name">{o.label}</span>
        {o.value === req.value && <Icon name="check" />}
      </button>
    ));
  } else {
    const m = boardMeta(ws, req.id);
    if (!m) return null;
    const single = allBoards(ws).length <= 1;
    body = (
      <>
        <div className="ctx-title">
          <span className="pj-bic" style={{ ['--pc' as string]: m.project.color }}>
            <Icon name="board" />
          </span>
          <b>{m.board.name}</b>
        </div>
        <button type="button" data-do="open" onClick={go(() => openBoard(req.id))}>
          <Icon name="move" />
          打开白板
        </button>
        <button type="button" data-do="edit-board" onClick={go(() => editBoard(req.id))}>
          <Icon name="edit" />
          编辑白板
        </button>
        <button type="button" data-do="copy-board-prompt" onClick={go(() => copyBoardPrompt(req.id))}>
          <Icon name="prompt" />
          复制为提示词
        </button>
        <button type="button" data-do="copy-path" onClick={go(() => copyBoardEnv(req.id))}>
          <Icon name="copy" />
          复制 <code>BOARD={req.id}</code>
        </button>
        <div className="ctx-sep" />
        <button type="button" data-do="delete-board" className="danger" disabled={single} onClick={go(() => deleteBoard(req.id))}>
          <Icon name="trash" />
          删除白板
        </button>
      </>
    );
  }
  return (
    <div
      className={cx('ctx', req.kind === 'prompts' && 'ctx-prompts', req.kind === 'move' && 'ctx-move', req.kind === 'select' && 'ctx-select', pos && 'on')}
      ref={ref}
      role={req.kind === 'select' ? 'listbox' : 'menu'}
      style={{ ...(pos ?? { left: -9999, top: -9999 }), ...(req.kind === 'select' && req.anchor ? { minWidth: req.anchor.width } : null) }}
      onKeyDown={e => {
        const bs = [...(ref.current?.querySelectorAll<HTMLElement>('[data-do]:not([disabled])') ?? [])];
        const i = bs.indexOf(document.activeElement as HTMLElement);
        if (req.kind === 'select') {
          // 选择框：Home / End 到头尾；按字符跳到以它开头的那一项（和系统下拉一样）
          if (e.key === 'Home' || e.key === 'End') {
            e.preventDefault();
            bs[e.key === 'Home' ? 0 : bs.length - 1]?.focus();
          } else if (e.key === 'Tab') {
            e.preventDefault();
            closeCtx();
          } else if (e.key.length === 1 && e.key !== ' ' && !e.metaKey && !e.ctrlKey && !e.altKey) {
            const k = e.key.toLowerCase();
            const hit = [...bs.slice(i + 1), ...bs.slice(0, i + 1)].find(b => b.textContent?.trim().toLowerCase().startsWith(k));
            hit?.focus();
          }
        }
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          bs[(i + 1) % bs.length]?.focus();
        }
        if (e.key === 'ArrowUp') {
          e.preventDefault();
          bs[(i - 1 + bs.length) % bs.length]?.focus();
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          closeCtx();
        }
      }}
    >
      {body}
    </div>
  );
}

/* ============================ 卡片详情「移动到」 ============================ */
/* 列出当前状态以外的工作流状态；不允许的（被 agent 持有、锁定状态）标出原因，点了给出与拖拽相同的提示（lib/drag.ts moveTo） */

function MoveMenu({ id, go }: { id: number; go: (fn: () => void) => () => void }) {
  const board = useData(s => s.board);
  const card = board?.cards[id];
  if (!board || !card) return null;
  const f = boardFlow(board);
  const targets = f.list.filter(s => s.key !== card.column);
  return (
    <>
      <div className="ctx-title">
        <Icon name="move" />
        <b>把 #{card.id} 移动到</b>
      </div>
      {card.holder && <p className="sort-hint">{card.holder} 正持有这张卡片，只有持有者能移动</p>}
      {targets.map(s => {
        const veto = vetoFor(card, s.key, f);
        return (
          <button
            key={s.key}
            type="button"
            data-do={`move-${s.key}`}
            className={cx('mv-item', veto && 'vetoed')}
            aria-disabled={!!veto}
            title={veto?.message}
            onClick={go(() => moveTo(card.id, s.key))}
          >
            <i className="mv-dot" style={{ background: f.color(s.key) }} />
            <span className="mv-name">{s.name}</span>
            {veto ? (
              <span className="mv-note">
                <Icon name="lock" />
                {/* 被持有时标题下已经说过一次，行里只留锁 */}
                {veto.code !== 'NOT_HOLDER' && veto.title}
              </span>
            ) : (
              <span className="mv-count mono">{colCount(board, s.key)}</span>
            )}
          </button>
        );
      })}
      {!targets.length && <p className="sort-hint">工作流里没有别的状态</p>}
      {/* 放回「未整理」：只有人能移入（界面上就是人）；被持有时和别的状态一样不能移 */}
      {card.column !== INBOX && (
        <>
          <div className="ctx-sep" />
          <button
            type="button"
            data-do="move-inbox"
            className={cx('mv-item', card.holder && 'vetoed')}
            aria-disabled={!!card.holder}
            title={vetoFor(card, INBOX, f)?.message}
            onClick={go(() => moveTo(card.id, INBOX))}
          >
            <i className="mv-dot" style={{ background: 'var(--c-inbox)' }} />
            <span className="mv-name">{INBOX_NAME}</span>
            {card.holder ? (
              <span className="mv-note">
                <Icon name="lock" />
              </span>
            ) : (
              <span className="mv-count mono">{colCount(board, INBOX)}</span>
            )}
          </button>
        </>
      )}
    </>
  );
}

/* ============================ 提示词菜单 ============================ */
/* 白板页头「提示词」：每个可领取状态一条，点一下复制；默认单行摘要；带标签的提示词也按可领取状态各来一组 */

function PromptsMenu({ path, go }: { path: string; go: (fn: () => void) => () => void }) {
  const ws = useData(s => s.ws);
  const board = useData(s => s.board);
  const flow = boardFlow(board);
  // 可领取状态按工作流顺序；第一个也是「拆解 / 上传」的领取起点
  const claimables = flow.claimables.length ? flow.claimables.map(s => s.name) : [flow.home?.name ?? '待办'];
  const labels = ws?.projects.find(p => p.key === path.split('/')[0])?.labels ?? [];
  const list = quickPrompts(path, claimables);
  return (
    <>
      <div className="ctx-title">
        <Icon name="prompt" />
        <b>复制给 agent 的提示词</b>
      </div>
      <div className="qp-list">
        {list.map(q => (
          <button key={q.id} type="button" data-do={`quick-${q.id}`} className="qp-item" title={q.text} onClick={go(() => copyQuickPrompt(q))}>
            <span className="qp-title">{q.title}</span>
            <code className="qp-text">{q.text}</code>
          </button>
        ))}
      </div>
      {labels.length ? (
        claimables.map(name => (
          <div key={name} className="qp-item qp-labels">
            <span className="qp-title">做「{name}」里带指定标签的任务</span>
            <div className="qp-chips">
              {labels.map(l => {
                const q = quickPrompts(path, claimables, l.name).find(x => x.kind === 'label' && x.status === name)!;
                return (
                  <button key={l.id} type="button" data-do={`quick-label-${name}-${l.id}`} data-tip={q.text} onClick={go(() => copyQuickPrompt(q))}>
                    <LabelChip label={l} />
                  </button>
                );
              })}
            </div>
          </div>
        ))
      ) : (
        <div className="qp-item qp-labels">
          <span className="qp-empty">项目里还没有标签</span>
        </div>
      )}
    </>
  );
}
