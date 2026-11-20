/* 白板勾选卡片、批量打标签的界面（lib/boardSelect）：侧栏「批量」按钮、块标题的勾选框、底部批量条、卡片上的勾选圈 */
import { Fragment } from 'react';
import { blockState, bulkLabel, bulkMove, clearSelect, moveTargets, selectIds, startSelect, toggleBlock, useBoardSel } from '../lib/boardSelect';
import { boardFlow, INBOX } from '../lib/schema';
import type { BoardSnapshot, Label } from '../lib/types';
import { cx } from '../lib/util';
import { useCurrent, useData } from '../store/data';
import { closeCtx, openCtx, openPicker, setBoardSheet, toast, useUI } from '../store/ui';
import { Tip } from './bits';
import { cardHit, filterActive } from './Card';
import { Icon } from './Icon';

/** 看得见的卡片（ids 给了就只在这些里挑）：不在未整理、不是还在新建的、没被筛选挡住 */
function shownIds(board: BoardSnapshot | null, labels: Label[], filter: string, sel: number[], ids?: number[]) {
  if (!board) return [];
  const q = filter.trim().toLowerCase();
  const on = filterActive(q, sel);
  return (ids ?? Object.keys(board.cards).map(Number))
    .map(id => board.cards[id])
    .filter(c => c && c.id > 0 && c.column !== INBOX && (!on || cardHit(c, q, sel, labels)))
    .map(c => c.id);
}

/** 侧栏「标签」标题旁：进入 / 退出选择。手机上在底部抽屉里，点了先收起抽屉，好去点卡片 */
export function SideSelect() {
  const on = useBoardSel(s => s.on);
  return (
    <Tip text={on ? '退出选择 · esc' : '选几张卡片一起打标签 · 也可以按住 ⌘ / Shift 点卡片'}>
      <button
        type="button"
        className={cx('side-select', on && 'on')}
        aria-pressed={on}
        onClick={() => {
          if (on) return clearSelect();
          startSelect();
          setBoardSheet(false);
        }}
      >
        <Icon name="checkCircle" />
        {on ? '完成' : '批量'}
      </button>
    </Tip>
  );
}

/** 块标题上的勾选框：选择里才出现，画出这一块一张没选 / 选了一部分 / 全选上 */
export function ColSelect({ ids, board, labels }: { ids: number[]; board: BoardSnapshot; labels: Label[] }) {
  const on = useBoardSel(s => s.on);
  const picked = useBoardSel(s => s.ids);
  const filter = useUI(s => s.filter);
  const sel = useUI(s => s.labelSel);
  if (!on) return null;
  const mine = shownIds(board, labels, filter, sel, ids);
  if (!mine.length) return null;
  const state = blockState(mine, picked);
  const tip = state === 'all' ? '取消这一块' : filterActive(filter, sel) ? '选上这一块筛出的卡片' : '选上这一块';
  return (
    <Tip text={tip}>
      <button
        type="button"
        className={cx('col-select', state)}
        aria-label={tip}
        aria-pressed={state === 'all' ? true : state === 'some' ? 'mixed' : false}
        onPointerDown={e => e.stopPropagation()}
        onClick={() => toggleBlock(mine)}
      >
        <span className="sel-box">{state === 'some' ? <i className="sel-dash" /> : <Icon name="check" sw={2.4} />}</span>
      </button>
    </Tip>
  );
}

/** 卡片前面的勾选圈（选择里才有） */
export function CardCheck({ on }: { on: boolean }) {
  return (
    <span className={cx('card-check', on && 'on')} aria-hidden>
      <Icon name="check" sw={2.4} />
    </span>
  );
}

/** 选择里从底部浮出的批量条（和「未整理」的同一套样子） */
export function BoardBulkBar() {
  const on = useBoardSel(s => s.on);
  const raw = useBoardSel(s => s.ids);
  const board = useData(s => s.board);
  const cur = useCurrent();
  const filter = useUI(s => s.filter);
  const sel = useUI(s => s.labelSel);
  const labels = cur?.project.labels ?? [];
  // 被别处删掉、移进未整理的卡片不算
  const ids = raw.filter(id => board?.cards[id] && board.cards[id].column !== INBOX);
  const n = ids.length;

  const pickLabels = (rect: DOMRect) => {
    if (!cur || !n) return;
    const cards = board?.cards ?? {};
    openPicker({
      rect,
      project: cur.project.key,
      selected: labels.filter(l => ids.every(id => cards[id]?.labels.includes(l.id))).map(l => l.id),
      onToggle: async (lid, add) => {
        const r = await bulkLabel(ids, lid, add);
        // 标签可能是在选择器里刚新建的：从最新的工作区里取名称
        const name = useData.getState().ws?.projects.find(p => p.key === cur.project.key)?.labels.find(l => l.id === lid)?.name ?? '';
        const k = r.changed.length;
        const skip = r.skipped.length;
        if (k > 0 || skip > 0)
          toast({
            type: 'success',
            title: add ? `${k} 张加上了「${name}」` : `${k} 张去掉了「${name}」`,
            text: skip > 0 ? `${skip} 张${add ? '本来就有' : '本来就没有'}` : undefined,
          });
      },
    });
  };

  return (
    <div className={cx('ib-bulk board-bulk', on && 'on')} role="toolbar" aria-label="批量操作" aria-hidden={!on} inert={!on || undefined}>
      <span className="ib-bulk-n">
        {n ? (
          <>
            <b className="mono" key={n}>
              {n}
            </b>
            张已选
          </>
        ) : (
          <span className="board-bulk-hint">点卡片选择 · Shift 连选</span>
        )}
      </span>
      <i className="ib-bulk-sep" />
      <button type="button" className="btn sm ghost" onClick={() => selectIds(shownIds(board, labels, filter, sel))}>
        <Icon name="checkCircle" />
        <span className="ib-bl">{filterActive(filter.trim(), sel) ? '全选筛出的' : '全选'}</span>
      </button>
      <button type="button" className="btn sm ghost" data-label-picker disabled={!n} onClick={e => pickLabels(e.currentTarget.getBoundingClientRect())}>
        <Icon name="tag" />
        <span className="ib-bl">打标签</span>
      </button>
      <button
        type="button"
        className="btn sm ghost bulk-move-btn"
        aria-haspopup="menu"
        disabled={!n}
        onClick={e => {
          if (useUI.getState().ctx?.kind === 'bulk-move') return closeCtx();
          openCtx({ kind: 'bulk-move', id: '', x: 0, y: 0, anchor: e.currentTarget.getBoundingClientRect() });
        }}
      >
        <Icon name="move" />
        <span className="ib-bl">移动到</span>
      </button>
      <Tip text="退出选择 · esc">
        <button type="button" className="icon-btn sm" aria-label="退出选择" data-tip-pos="top" onClick={clearSelect}>
          <Icon name="close" />
        </button>
      </Tip>
    </div>
  );
}

/** 批量条「移动到」的菜单（在 CtxMenu 里画）：本白板的每个状态和「未整理」，写能移几张、跳过几张；锁定状态整行不能选 */
export function BulkMoveMenu({ go }: { go: (fn: () => void) => () => void }) {
  const board = useData(s => s.board);
  const raw = useBoardSel(s => s.ids);
  if (!board) return null;
  const ids = raw.filter(id => board.cards[id] && board.cards[id].column !== INBOX);
  const targets = moveTargets(board, ids);
  return (
    <>
      <div className="ctx-title">
        <Icon name="move" />
        <b>把 {ids.length} 张移动到</b>
      </div>
      {targets.map(t => {
        const inbox = t.key === INBOX;
        const skip = [t.here && `${t.here} 张已在这里`, t.held && `${t.held} 张被持有`].filter(Boolean).join(' · ');
        const off = !t.ids.length;
        const why = t.locked ?? (t.here === ids.length ? '都已经在这里' : off ? '选中的都被 agent 持有' : '');
        return (
          <Fragment key={t.key}>
            {inbox && <div className="ctx-sep" />}
            <button
              type="button"
              data-do={`move-${t.key}`}
              className={cx('mv-item', off && 'vetoed')}
              aria-disabled={off}
              title={t.locked ? `「${t.name}」已锁定：卡片只能由 agent 通过 board claim 领取进入` : skip ? `跳过：${skip}` : undefined}
              onClick={off ? undefined : go(() => void bulkMove(t))}
            >
              <i className="mv-dot" style={{ background: inbox ? 'var(--c-inbox)' : statusDot(board, t.key) }} />
              <span className="mv-name">{t.name}</span>
              {off ? (
                <span className="mv-note">
                  {t.locked && <Icon name="lock" />}
                  {why}
                </span>
              ) : (
                <span className="mv-note">
                  <b className="mono">{t.ids.length}</b> 张{skip && <span className="mv-skip"> · 跳过 {t.here + t.held}</span>}
                </span>
              )}
            </button>
          </Fragment>
        );
      })}
    </>
  );
}

const statusDot = (board: BoardSnapshot, key: string) => boardFlow(board).color(key);
