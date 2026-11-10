/* 右键 / ⋯ 下拉菜单：卡片、状态、项目、白板菜单，以及「移动到」「排序」「提示词」子菜单 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import {
  copyBoardEnv, copyBoardPrompt, copyQuickPrompt, deleteBoard, deleteProject, editBoard, editProject, newBoard, openBoard, setProjectColor,
} from '../../lib/actions';
import { moveTo, vetoFor } from '../../lib/drag';
import { quickPrompts } from '../../lib/prompt';
import { SORTS, sortKey, type SortSpec } from '../../lib/sort';
import { boardFlow } from '../../lib/schema';
import { cx } from '../../lib/util';
import { allBoards, boardMeta, useData } from '../../store/data';
import { closeCtx, manageLabels, setSort, useUI } from '../../store/ui';
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
    setPos({
      left: Math.max(8, Math.min(req.anchor ? r.right - w : r.left, innerWidth - w - 8)),
      top: r.bottom + h + 8 > innerHeight ? Math.max(8, r.top - h - 6) : r.bottom + 6,
    });
    requestAnimationFrame(() => m.querySelector<HTMLElement>('button[data-do]:not([disabled])')?.focus({ preventScroll: true }));
  }, [req]);

  useEffect(() => {
    if (!req) return;
    const down = (e: PointerEvent) => !(e.target as HTMLElement).closest('.ctx, .prompts-btn, .col-sort-btn, .d-move') && closeCtx();
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
  } else if (req.kind === 'sort') {
    body = <SortMenu col={req.id} go={go} />;
  } else if (req.kind === 'move') {
    body = <MoveMenu id={Number(req.id)} go={go} />;
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
      className={cx('ctx', req.kind === 'prompts' && 'ctx-prompts', req.kind === 'sort' && 'ctx-sort', req.kind === 'move' && 'ctx-move', pos && 'on')}
      ref={ref}
      role="menu"
      style={pos ?? { left: -9999, top: -9999 }}
      onKeyDown={e => {
        const bs = [...(ref.current?.querySelectorAll<HTMLElement>('[data-do]:not([disabled])') ?? [])];
        const i = bs.indexOf(document.activeElement as HTMLElement);
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
              <span className="mv-count mono">{board.cols[s.key]?.length ?? 0}</span>
            )}
          </button>
        );
      })}
      {!targets.length && <p className="sort-hint">工作流里没有别的状态</p>}
    </>
  );
}

/* ============================ 状态块排序菜单 ============================ */
/* 一次性排序：只改这块的显示顺序，不写服务端、不改优先级（lib/sort.ts） */

function SortMenu({ col, go }: { col: string; go: (fn: () => void) => () => void }) {
  const path = useData(s => s.path);
  const board = useData(s => s.board);
  const key = sortKey(path, col);
  const cur = useUI(s => s.sorts[key]);
  const pick = (spec: SortSpec | null) => go(() => setSort(key, spec));
  return (
    <>
      <div className="ctx-title">
        <Icon name="sort" />
        <b>排序「{boardFlow(board).name(col)}」</b>
      </div>
      <p className="sort-hint">只改这块的显示顺序，优先级和 agent 的领取顺序不变</p>
      <button type="button" data-do="sort-none" className={cx(!cur && 'on')} onClick={pick(null)}>
        <span className="sort-name">优先级</span>
        {!cur && <Icon name="check" />}
      </button>
      {SORTS.map(o => {
        const on = cur?.by === o.by;
        return (
          <div key={o.by} className={cx('sort-row', on && 'on')}>
            <button type="button" data-do={`sort-${o.by}`} onClick={pick({ by: o.by, desc: on ? cur.desc : o.desc })}>
              <span className="sort-name">{o.name}</span>
              {!o.dir && <span className="sort-note">按项目标签顺序分组</span>}
              {on && !o.dir && <Icon name="check" />}
            </button>
            {o.dir && (
              <span className="sort-dir" role="group" aria-label={`${o.name}的方向`}>
                {[false, true].map(desc => (
                  <button
                    key={String(desc)}
                    type="button"
                    data-do={`sort-${o.by}-${desc ? 'desc' : 'asc'}`}
                    className={cx(on && cur.desc === desc && 'on')}
                    aria-pressed={on && cur.desc === desc}
                    onClick={pick({ by: o.by, desc })}
                  >
                    {desc ? '降序' : '升序'}
                  </button>
                ))}
              </span>
            )}
          </div>
        );
      })}
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

/* ============================ 标签选择器 ============================ */
