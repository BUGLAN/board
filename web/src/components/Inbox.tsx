/* ==========================================================================
   「未整理」页（PRD §10.5）：当前白板「未整理」里的全部条目摊开在主区，方便改、合并、整理
   - 按时间 / 来源 / 标签分组，一行一条；点一行原地展开编辑（标题、标签、描述，和卡片详情同一套编辑器）
   - 勾选（Shift 连选）后底部浮出批量条：合并成一条、标签、删除、整理到首个可领取状态
   - 合并先预览：选保留哪一条的编号，合并后的标题和描述可以先改再确认
   侧栏（Workbench 的 InboxSide）：记一条未整理、筛选、来源、放了多久
   写操作都走 store/data 的乐观更新：当帧生效，行在原位播离场，其余的行用弹簧补间补位
   - 拆解（PRD §10.5、§9.1）：展开的一行「派给 agent 拆解」；正在拆解的标「拆解中 · 机器 · agent」，展开后可以停止。
     拆出卡之后原条目不再单独占一行：拆出的卡成组，组头写「拆自 #ID 标题」，展开看原条目的描述，
     拆出的卡都还在未整理里时可以「撤销拆解」（拆出的卡进回收站，原条目回来）；整理走了之后原条目一直隐藏（lib/split）
   ========================================================================== */
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { create } from 'zustand';
import { openCard, triage, untriage } from '../lib/actions';
import { cardDispatch } from '../lib/hosts';
import { boardFlow, INBOX } from '../lib/schema';
import { canUndoSplit, kidsOf, hiddenEntries, unitCards, unitRep, unitsOf, type Unit } from '../lib/split';
import { deleteCards, undoDelete } from '../lib/trash';
import type { Actor, Card, Label } from '../lib/types';
import { actorName, ago, autosize, cx, isMac, pref, reduceMotion, replay } from '../lib/util';
import { addLabel, cardBusy, cardKey, entryIsNew, exec, isDraft, markAllSeen, markSeen, mergePatch, movePatch, newEntryCount, removeLabel, renderKey, serverNow, useCardBusy, useCurrent, useData } from '../store/data';
import { confirm, openCreator, openCtx, closeCtx, openDispatch, openPicker, openSession, overlayOpen, reveal, setFilter, toast, toastError, useUI, type Flash } from '../store/ui';
import { Avatar, DescTabs, LabelChip, LabelChips, Markdown, Title, useNow } from './bits';
import { matches } from './Card';
import { CardLabels, DescEditor, TitleEditor } from './CardDetail';
import { Icon } from './Icon';
import { Skeleton } from './Skeleton';
import { cancel } from './hosts/Dispatch';

/* ------------------------------ 状态 ------------------------------ */

type Group = 'time' | 'source' | 'label';
interface MergeReq {
  ids: number[];
  keep: number;
  /** 改过的标题、描述（没改时按选的保留条目自动拼） */
  draft: { title: string; body: string } | null;
}
interface InboxUI {
  /** 只看某个来源（srcKey） */
  src: string | null;
  group: Group;
  checked: number[];
  /** 展开编辑的那一行 */
  open: number | null;
  merge: MergeReq | null;
}
const GROUPS: Group[] = ['time', 'source', 'label'];
const savedGroup = pref.get<Group>('inboxGroup', 'time');
export const useInbox = create<InboxUI>(() => ({ src: null, group: GROUPS.includes(savedGroup) ? savedGroup : 'time', checked: [], open: null, merge: null }));
const setInbox = (p: Partial<InboxUI>) => useInbox.setState(p);
const setGroup = (group: Group) => {
  setInbox({ group });
  pref.set('inboxGroup', group);
};

/* ------------------------------ 分组 ------------------------------ */

const DAY = 86400e3;
const STALE = 14 * DAY;
const BUCKETS = ['today', 'week', 'older', 'stale'] as const;
type Bucket = (typeof BUCKETS)[number];
const BUCKET_NAME: Record<Bucket, string> = { today: '今天', week: '这一周', older: '更早', stale: '放了两周以上' };

function bucketOf(t: number, now: number): Bucket {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  if (t >= start.getTime()) return 'today';
  if (now - t < 7 * DAY) return 'week';
  if (now - t < STALE) return 'older';
  return 'stale';
}

export const srcKey = (a: Actor) => `${a.kind}:${a.name}`;

interface GroupOf {
  key: string;
  label: string;
  stale?: boolean;
  /** 一条一行，或者拆自同一条的一组（lib/split） */
  units: Unit[];
  /** 展平后的每一行（勾选、连选、计数用） */
  rows: Card[];
}

const groupOf = (key: string, label: string, units: Unit[], stale?: boolean): GroupOf => ({ key, label, stale, units, rows: units.flatMap(unitCards) });

/** 分组：拆自同一条的一组按组里最新的那张算（时间、来源、标签都是） */
function groupsOf(units: Unit[], group: Group, now: number, labels: Label[]): GroupOf[] {
  if (group === 'time')
    return BUCKETS.map(k => groupOf(k, BUCKET_NAME[k], units.filter(u => bucketOf(unitRep(u).createdAt, now) === k), k === 'stale')).filter(g => g.rows.length);
  if (group === 'source') {
    const map = new Map<string, Unit[]>();
    for (const u of units) map.set(srcKey(unitRep(u).createdBy), [...(map.get(srcKey(unitRep(u).createdBy)) ?? []), u]);
    // 条目多的来源在前，人自己记的放最后
    return [...map.entries()]
      .map(([key, us]) => groupOf(key, actorName(unitRep(us[0]).createdBy), us))
      .sort((x, y) => Number(x.key.startsWith('human')) - Number(y.key.startsWith('human')) || y.rows.length - x.rows.length);
  }
  // 按标签：按项目里标签的顺序，一条放在它的第一个标签下
  const first = (u: Unit) => labels.find(l => unitRep(u).labels.includes(l.id))?.id;
  const out: GroupOf[] = labels.map(l => groupOf(`l${l.id}`, l.name, units.filter(u => first(u) === l.id))).filter(g => g.rows.length);
  const none = units.filter(u => first(u) == null);
  return none.length ? [...out, groupOf('none', '没有标签', none)] : out;
}

const NO_LABELS: Label[] = [];

const NO_PARENTS: Record<number, Card> = {};

/** 当前白板「未整理」里显示的条目（最新的在前；已经拆出卡的原条目不算，见 lib/split）和筛选后的列表；快照还没到时为 null。
    parents：被隐藏的原条目，拆出的卡按它成组；筛选词对上原条目时，整组都留着 */
function useInboxCards() {
  const board = useData(s => s.board);
  const cur = useCurrent();
  const q = useUI(s => s.filter.trim().toLowerCase());
  const src = useInbox(s => s.src);
  const labels = cur?.project.labels ?? NO_LABELS;
  const { all, parents } = useMemo(() => {
    if (!board) return { all: null, parents: NO_PARENTS };
    const inbox = board.cols[INBOX] ?? [];
    const hidden = hiddenEntries(board.cards, inbox);
    const parents: Record<number, Card> = {};
    for (const id of hidden) if (board.cards[id]) parents[id] = board.cards[id];
    const all = inbox
      .filter(id => !hidden.has(id))
      .map(id => board.cards[id])
      .filter(Boolean)
      .sort((a, b) => b.createdAt - a.createdAt || b.id - a.id);
    return { all, parents };
  }, [board]);
  const list = useMemo(() => {
    const hit = (c: Card) => matches(c, q, labels) || (c.splitFrom != null && !!parents[c.splitFrom] && matches(parents[c.splitFrom], q, labels));
    return all?.filter(c => (!src || srcKey(c.createdBy) === src) && hit(c)) ?? null;
  }, [all, parents, src, q, labels]);
  return { board, all, list, labels, parents };
}

/** 描述的第一行文字：跳过 Markdown 小标题（只有小标题时才用它），去掉列表、引用、勾选框的符号，图片和链接只留文字 */
function snippet(body: string) {
  const lines = body.split('\n').map(l => l.trim()).filter(Boolean);
  const text = (l: string) => l.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/^[-*#>\s]+|\[[ xX]\]\s*/g, '').trim();
  return lines.filter(l => !/^#{1,6}\s/.test(l)).map(text).find(Boolean) ?? lines.map(text).find(Boolean) ?? '';
}

/** 「可能是同一件事」：标题里共同出现的词（两个汉字或三个以上字母）至少两个，或者同一个标签再加一个共同的词 */
function related(c: Card, all: Card[]) {
  const words = (t: string) => new Set(t.match(/[一-龥]{2}|[a-z]{3,}/gi)?.map(w => w.toLowerCase()) ?? []);
  const a = words(c.title);
  return new Set(
    all
      .filter(o => o.id !== c.id)
      .filter(o => {
        const b = words(o.title);
        const common = [...a].filter(w => b.has(w)).length;
        return common >= 2 || (common >= 1 && o.labels.some(l => c.labels.includes(l)));
      })
      .map(o => o.id),
  );
}

/* ------------------------------ 动作 ------------------------------ */

/** 行离开时在原位播的动画：整理 → 往右滑走，删除 → 原地淡出，合并 → 往上收 */
const exits = new Map<number, 'triage' | 'delete' | 'merge'>();

async function doTriage(ids: number[]) {
  const ok = ids.filter(id => !isDraft(id));
  if (ok.length === 1) {
    exits.set(ok[0], 'triage');
    return triage(ok[0]);
  }
  const home = boardFlow(useData.getState().board).home;
  const to = home?.key ?? 'todo';
  ok.forEach(id => exits.set(id, 'triage'));
  const all = Promise.allSettled(ok.map(id => exec({ op: 'move', id, column: to }, movePatch(id, to))));
  // 当帧提示；撤销等这几次移动都确认后，把成功的放回「未整理」
  const undo = () => void all.then(rs => untriage(ok.filter((_, i) => rs[i].status === 'fulfilled')));
  toast({ type: 'success', title: `${ok.length} 条已整理到「${home?.name ?? '待办'}」`, text: ok.map(i => `#${i}`).join('、'), action: { label: '撤销', fn: undo } });
  const bad = (await all).find(r => r.status === 'rejected') as PromiseRejectedResult | undefined;
  if (bad) toastError(bad.reason);
}

async function doDelete(ids: number[], cards: Card[]) {
  const ok = ids.filter(id => !isDraft(id));
  if (!ok.length) return;
  const one = cards.find(c => c.id === ok[0]);
  const yes = await confirm(
    ok.length === 1
      ? {
          title: `删除 #${ok[0]}？`,
          text: `「${one?.title ?? ''}」会移进回收站（侧栏「回收站」），30 天内可以恢复；永久删除前评论和附件都还在。`,
          ok: '删除',
        }
      : {
          title: `删除 ${ok.length} 条？`,
          text: `${ok.map(i => `#${i}`).join('、')} 会移进回收站，30 天内可以恢复；永久删除前评论和附件都还在。`,
          ok: `删除 ${ok.length} 条`,
        },
  );
  if (!yes) return;
  setInbox({ checked: useInbox.getState().checked.filter(c => !ok.includes(c)) });
  ok.forEach(id => exits.set(id, 'delete'));
  // 提示里可以「撤销」：放回「未整理」原来的位置
  await deleteCards(ok, { titles: Object.fromEntries(cards.map(c => [c.id, c.title])) });
}

/** 合并后的描述（服务端不给描述时按同样的格式拼）：留下的那条原文在前，每条来源一节，节末写明合并自哪一条 */
function mergedBody(keep: Card, others: Card[]) {
  const parts = [keep.body.trim()];
  for (const o of others) {
    const who = o.createdBy.kind === 'agent' ? ` · ${o.createdBy.name}` : '';
    parts.push([`## ${o.title}`, o.body.trim(), `> 合并自 #${o.id}${who}`].filter(Boolean).join('\n\n'));
  }
  return parts.filter(Boolean).join('\n\n');
}

/** 打开合并预览：默认保留最早记下的那条（编号可能已经被别处引用）；keep 给了就保留它（「并到…」选的目标） */
function openMerge(ids: number[], keep?: number) {
  const board = useData.getState().board;
  const cards = ids.map(id => board?.cards[id]).filter(Boolean) as Card[];
  if (cards.length < 2) return;
  const k = keep ?? [...cards].sort((a, b) => a.createdAt - b.createdAt || a.id - b.id)[0].id;
  setInbox({ merge: { ids: cards.map(c => c.id), keep: k, draft: null } });
}

/** 全部标为已读（PRD §10.5）：当帧去掉所有的点，之后接着记下的条目照常算新；提示里写标之前有几条 */
async function doMarkAll(sent: (at: number | null) => void) {
  const s = useData.getState();
  const n = newEntryCount(s.path, s.board, null);
  if (!n) return;
  // 服务端快照回来之前，按这一页自己的时刻算：标之前的都不算新
  const at = serverNow();
  sent(at);
  try {
    await markAllSeen();
    // 服务端确认后由服务端的 seen_at 说了算；这一刻留到这一页关掉
    toast({ type: 'success', title: `${n} 条标为已读`, duration: 2000 });
  } catch (e) {
    sent(null);
    toastError(e);
  }
}

async function doMerge(req: MergeReq, value: { title: string; body: string }) {
  const others = req.ids.filter(id => id !== req.keep);
  setInbox({ merge: null, checked: [], open: null });
  others.forEach(id => exits.set(id, 'merge'));
  try {
    await exec({ op: 'merge', id: req.keep, from: others, title: value.title, body: value.body }, mergePatch(req.keep, others, value));
    toast({ type: 'success', title: `已合并成 #${req.keep}`, text: `${others.map(i => `#${i}`).join('、')} 的内容接在了描述后面` });
  } catch (e) {
    // 人改过的标题、描述不丢：带着它重新打开预览
    toastError(e);
    setInbox({ merge: { ...req, draft: value } });
  }
}

/* ------------------------------ 主区 ------------------------------ */

export function InboxPage() {
  const { board, all, list, labels, parents } = useInboxCards();
  const cur = useCurrent();
  const path = useData(s => s.path);
  const group = useInbox(s => s.group);
  const checked = useInbox(s => s.checked);
  const open = useInbox(s => s.open);
  const merge = useInbox(s => s.merge);
  const filter = useUI(s => s.filter);
  const src = useInbox(s => s.src);
  const reveal = useUI(s => s.reveal);
  const now = useNow(60000);
  const home = boardFlow(board).home?.name ?? '待办';
  const wrap = useRef<HTMLDivElement>(null);
  const lastCheck = useRef<number | null>(null);
  const groups = useMemo(() => (list ? groupsOf(unitsOf(list, parents), group, now, labels) : []), [list, parents, group, now, labels]);
  const flat = useMemo(() => groups.flatMap(g => g.rows), [groups]);
  const present = useMemo(() => new Set((all ?? []).map(c => renderKey(c.id))), [all]);
  useRowMotion(wrap, present, path);
  // 「全部标为已读」时记下这一刻：服务端快照回来之前，这之前记下的条目就不再画成新的
  const [sent, setSent] = useState<number | null>(null);
  const newCount = useMemo(() => (all ?? []).filter(c => entryIsNew(path, c) && (sent == null || c.createdAt > sent)).length, [all, path, sent]);

  // 换白板：勾选、展开、来源筛选清空
  useEffect(() => setInbox({ checked: [], open: null, src: null, merge: null }), [path]);
  // 已经不在「未整理」里的条目从勾选里去掉（整理走了、别的窗口删了）
  useEffect(() => {
    if (!all) return;
    const ids = new Set(all.map(c => c.id));
    const st = useInbox.getState();
    if (st.checked.some(c => !ids.has(c))) setInbox({ checked: st.checked.filter(c => ids.has(c)) });
    if (st.open != null && !ids.has(st.open)) setInbox({ open: null });
  }, [all]);

  // 定位（例如刚记下一条）：滚到那一行并闪一下
  useEffect(() => {
    if (!reveal) return;
    const el = wrap.current?.querySelector<HTMLElement>(`[data-ib="${renderKey(reveal.id)}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'nearest', behavior: reduceMotion() ? 'auto' : 'smooth' });
    if (reveal.pulse) replay(el, 'pulse');
  }, [reveal]);

  const check = (id: number, shift: boolean) => {
    const cur = useInbox.getState().checked;
    const on = !cur.includes(id);
    if (shift && lastCheck.current != null && flat.some(c => c.id === lastCheck.current)) {
      const a = flat.findIndex(c => c.id === lastCheck.current);
      const b = flat.findIndex(c => c.id === id);
      const range = flat.slice(Math.min(a, b), Math.max(a, b) + 1).map(c => c.id);
      setInbox({ checked: on ? [...new Set([...cur, ...range])] : cur.filter(c => !range.includes(c)) });
    } else setInbox({ checked: on ? [...cur, id] : cur.filter(c => c !== id) });
    lastCheck.current = id;
  };

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, select, [contenteditable]') || overlayOpen() || useData.getState().detailId != null || useInbox.getState().merge) return;
      const st = useInbox.getState();
      if (e.key === 'Escape' && (st.checked.length || st.open != null)) {
        e.preventDefault();
        setInbox(st.checked.length ? { checked: [] } : { open: null });
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && flat.length) {
        e.preventDefault();
        setInbox({ checked: flat.filter(c => !isDraft(c.id)).map(c => c.id) });
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [flat]);

  const count = all?.length ?? null;
  return (
    <div className="page page-inbox">
      <div className="ib-scroll">
        <div className="ib-wrap" ref={wrap}>
          <header className="ib-head">
            <div>
              <h1>
                未整理
                {count != null && <span className="ib-total mono">{count}</span>}
              </h1>
              {/* agent 新记下、人还没看过的条数；没有时不占地方 */}
              {all && newCount > 0 && (
                <div className="ib-newline">
                  <span className="ib-new mono">{newCount} 条新的</span>
                  <button type="button" className="ib-link ib-markall" onClick={() => void doMarkAll(setSent)}>
                    全部标为已读
                  </button>
                </div>
              )}
              <p>agent 和你记下的想法、需求。点一行展开修改；勾选几条一起合并、打标签，或整理进「{home}」。</p>
            </div>
            <DescTabs
              mode={group}
              setMode={m => setGroup(m as Group)}
              modes={[
                ['time', '按时间'],
                ['source', '按来源'],
                ['label', '按标签'],
              ]}
            />
          </header>

          {/* 手机上没有侧栏：筛选和「记一条」放在页头下面 */}
          <div className="ib-mtools">
            <label className={cx('side-filter', filter && 'has-value')}>
              <Icon name="filter" />
              <input value={filter} placeholder="筛选标题、描述、#编号" spellCheck={false} autoComplete="off" onChange={e => setFilter(e.target.value)} />
              <button type="button" className="filter-clear" aria-label="清除筛选" onClick={() => setFilter('')}>
                ×
              </button>
            </label>
            <button className="btn sm" type="button" onClick={() => openCreator('inbox')}>
              <Icon name="plus" />
              记一条
            </button>
          </div>

          {/* 快照还没到：按工作区里的计数放骨架行；计数是 0 就什么都不放 */}
          {!list && (cur?.board.counts[INBOX] ?? 3) > 0 && <RowsSkeleton n={Math.min(cur?.board.counts[INBOX] ?? 3, 5)} />}
          {groups.map(g => (
            <GroupSection
              key={`${group}-${g.key}`}
              g={g}
              checked={checked}
              open={open}
              now={now}
              labels={labels}
              all={all ?? []}
              home={home}
              onCheck={check}
            />
          ))}
          {list && !list.length && (
            <div className="ib-empty">
              {all?.length ? (
                <>
                  <b>没有符合的条目</b>
                  <span>换个词试试，或者清除筛选。</span>
                  <button
                    className="btn sm"
                    type="button"
                    onClick={() => {
                      setFilter('');
                      setInbox({ src: null });
                    }}
                  >
                    清除筛选
                  </button>
                </>
              ) : (
                <>
                  <div className="ie-art" aria-hidden="true">
                    <i />
                    <i />
                    <i />
                  </div>
                  <b>全部整理完了</b>
                  <span>
                    agent 用 <code>board new --inbox</code> 记下的想法、需求会出现在这里，
                    <br />
                    整理清楚后再放进「{home}」。
                  </span>
                </>
              )}
            </div>
          )}
          {(filter || src) && list && list.length > 0 && (
            <p className="ib-filtered">
              筛选后 {list.length} / {all?.length} 条 ·{' '}
              <button
                type="button"
                className="ib-link"
                onClick={() => {
                  setFilter('');
                  setInbox({ src: null });
                }}
              >
                清除筛选
              </button>
            </p>
          )}
        </div>
      </div>
      <BulkBar checked={checked} cards={all ?? []} home={home} />
      {merge && <MergeSheet key={`${merge.ids.join(',')}`} req={merge} />}
    </div>
  );
}

function RowsSkeleton({ n }: { n: number }) {
  return (
    <div className="ib-grp" aria-busy="true">
      <div className="ib-gh">
        <Skeleton w={48} h={10} />
      </div>
      <div className="ib-rows">
        {Array.from({ length: n }, (_, i) => (
          <div key={i} className="ib-line sk">
            <Skeleton w={16} h={16} r={5} />
            <Skeleton w={28} h={10} />
            <Skeleton w={20} h={20} r={999} />
            <Skeleton w={['48%', '62%', '36%', '54%'][i % 4]} h={11} />
          </div>
        ))}
      </div>
    </div>
  );
}

function GroupSection({
  g, checked, open, now, labels, all, home, onCheck,
}: { g: GroupOf; checked: number[]; open: number | null; now: number; labels: Label[]; all: Card[]; home: string; onCheck: (id: number, shift: boolean) => void }) {
  const ids = g.rows.filter(c => !isDraft(c.id)).map(c => c.id);
  const allOn = ids.length > 0 && ids.every(id => checked.includes(id));
  const some = !allOn && ids.some(id => checked.includes(id));
  return (
    <section className="ib-grp" data-group={g.key}>
      <div className={cx('ib-gh', g.stale && 'stale')}>
        <Check
          on={allOn}
          some={some}
          label={allOn ? `取消勾选「${g.label}」` : `勾选「${g.label}」这一组`}
          onToggle={() => {
            const cur = useInbox.getState().checked;
            setInbox({ checked: allOn ? cur.filter(c => !ids.includes(c)) : [...new Set([...cur, ...ids])] });
          }}
        />
        {g.stale && <Icon name="timer" />}
        <span>{g.label}</span>
        <span className="ib-gh-n mono">{g.rows.length}</span>
        {g.stale && <span className="ib-gh-hint">还要做吗？不做就删掉</span>}
      </div>
      <div className="ib-rows">
        {g.units.map(u =>
          u.kind === 'one' ? (
            <Row key={renderKey(u.card.id)} card={u.card} now={now} labels={labels} all={all} home={home} checked={checked.includes(u.card.id)} open={open === u.card.id} onCheck={onCheck} />
          ) : (
            <SplitBundle key={`split-${u.parent.id}`} parent={u.parent} cards={u.cards} checked={checked} open={open} now={now} labels={labels} all={all} home={home} onCheck={onCheck} />
          ),
        )}
      </div>
    </section>
  );
}

/** 勾选框：自己画的方块（选中打勾、部分选中一道横线），点的时候弹一下 */
function Check({ on, some, label, onToggle }: { on: boolean; some?: boolean; label: string; onToggle: (shift: boolean) => void }) {
  return (
    <label className={cx('ib-check', on && 'on', some && 'some')} onClick={e => e.stopPropagation()}>
      <input
        type="checkbox"
        checked={on}
        aria-label={label}
        onChange={() => {}}
        onClick={e => {
          e.stopPropagation();
          onToggle(e.shiftKey);
        }}
      />
      <i />
    </label>
  );
}

/** 放了多久：两周以上标成警告色 */
function Age({ card: c, now }: { card: Card; now: number }) {
  const d = Math.floor((now - c.createdAt) / DAY);
  if (now - c.createdAt >= STALE)
    return (
      <span className="ib-age stale" title={`放了 ${d} 天：还要做吗？`}>
        <Icon name="timer" />
        {d} 天
      </span>
    );
  return <span className="ib-age">{d >= 1 ? `${d} 天前` : ago(c.createdAt, now)}</span>;
}

/* ------------------------------ 拆解 ------------------------------ */

/** 撤销拆解：拆出的卡（都还在未整理里）当帧删掉、进回收站，原条目随之回到列表里并闪一下；提示里可以把拆出的卡恢复回来 */
async function undoSplit(parent: Card) {
  const s = useData.getState();
  const cards = s.board?.cards;
  if (!canUndoSplit(cards, parent.id)) return;
  const kids = (kidsOf(cards).get(parent.id) ?? []).filter(id => !isDraft(id));
  const path = s.path;
  kids.forEach(id => exits.set(id, 'delete'));
  setInbox({ checked: useInbox.getState().checked.filter(c => !kids.includes(c)) });
  const done = deleteCards(kids, { silent: true });
  // 原条目当帧出现在列表里：滚到它并闪一下
  requestAnimationFrame(() => reveal(parent.id));
  if (!(await done)) return;
  toast({
    type: 'success',
    title: `已撤销拆解 #${parent.id}`,
    text: `拆出的 ${kids.map(k => `#${k}`).join('、')} 在回收站里，「${parent.title}」回到了未整理`,
    action: { label: '恢复拆出的卡', fn: () => void undoDelete(path, kids) },
    duration: 6000,
  });
}

type Dispatched = NonNullable<ReturnType<typeof cardDispatch>>;

/** 正在拆解（排队拆解）的说明条：机器、agent，运行中可以看会话，可以停止或取消 */
function SplitNote({ d }: { d: Dispatched }) {
  return (
    <div className="ib-split-note" role="status">
      <Icon name="dispatch" />
      <span>
        <b>{d.state === 'queued' ? '排队拆解' : '拆解中'}</b> · {d.host} · {d.agentLabel}
        {d.stopBy && ' · 正在停止'}
      </span>
      <span className="spacer" />
      {d.state === 'running' && (
        <button className="btn sm ghost" type="button" onClick={() => openSession({ id: d.session || d.id, card: d.card, known: [d] })}>
          <Icon name="log" />
          会话
        </button>
      )}
      {!d.stopBy && (
        <button className="btn sm danger" type="button" onClick={() => void cancel(d)}>
          <Icon name={d.state === 'queued' ? 'close' : 'stop'} />
          {d.state === 'queued' ? '取消拆解' : '停止'}
        </button>
      )}
    </div>
  );
}

function SplittingMark({ d }: { d: Dispatched }) {
  return (
    <span className={cx('ib-mark ib-splitting', d.state === 'queued' && 'queued')} data-tip={d.state === 'queued' ? '排队等机器有空位' : `${d.name ?? ''} 正在拆这一条`}>
      <i aria-hidden="true" />
      {d.state === 'queued' ? '排队拆解' : '拆解中'} · {d.host} · {d.agentLabel}
    </span>
  );
}

/** 这一行的拆解标记：拆解中（排队拆解）· 机器 · agent；拆自 #ID（在拆解组里时组头已经写了，不再标） */
function SplitMarks({ card: c, inBundle }: { card: Card; inBundle?: boolean }) {
  const d = useData(s => cardDispatch(s.ws, s.path, c.id));
  return (
    <>
      {d && <SplittingMark d={d} />}
      {c.splitFrom != null && !inBundle && (
        <span className="ib-mark ib-from" data-tip={`从 #${c.splitFrom} 拆出来的`}>
          拆自 #{c.splitFrom}
        </span>
      )}
    </>
  );
}

/** 拆解组：组头「拆自 #ID 标题」（点开看原条目的描述）、勾选整组、撤销拆解；下面是拆出的、还在未整理里的卡 */
function SplitBundle({
  parent: p, cards, checked, open, now, labels, all, home, onCheck,
}: { parent: Card; cards: Card[]; checked: number[]; open: number | null; now: number; labels: Label[]; all: Card[]; home: string; onCheck: (id: number, shift: boolean) => void }) {
  const [exp, setExp] = useState(false);
  const d = useData(s => cardDispatch(s.ws, s.path, p.id));
  const undoable = useData(s => canUndoSplit(s.board?.cards, p.id));
  const busy = useData(s => [p.id, ...cards.map(c => c.id)].some(id => !!s.saving[cardKey(s.path, id)]));
  const board = useData(s => s.board);
  const ids = cards.filter(c => !isDraft(c.id)).map(c => c.id);
  const allOn = ids.length > 0 && ids.every(id => checked.includes(id));
  const some = !allOn && ids.some(id => checked.includes(id));
  return (
    <div className={cx('ib-bundle', exp && 'open')} data-split={p.id}>
      <div className="ib-bh">
        <Check
          on={allOn}
          some={some}
          label={allOn ? `取消勾选拆自 #${p.id} 的这组` : `勾选拆自 #${p.id} 的这组`}
          onToggle={() => {
            const cur = useInbox.getState().checked;
            setInbox({ checked: allOn ? cur.filter(c => !ids.includes(c)) : [...new Set([...cur, ...ids])] });
          }}
        />
        <button type="button" className="ib-bh-main" aria-expanded={exp} onClick={() => setExp(!exp)}>
          <span className="ib-bh-k">拆自</span>
          <span className="ib-id mono">#{p.id}</span>
          <b>
            <Title text={p.title} />
          </b>
          <span className="ib-gh-n mono">{cards.length}</span>
          <Icon name="chevDown" className="ib-chev" />
        </button>
        {d && <SplittingMark d={d} />}
        {undoable && (
          <button
            type="button"
            className="btn sm ghost ib-undo-split"
            disabled={busy || !!d}
            data-tip={d ? '还在拆：先停止' : '删掉拆出的卡（进回收站），原条目回到未整理'}
            data-tip-pos="top"
            onClick={() => void undoSplit(p)}
          >
            <Icon name="replay" />
            撤销拆解
          </button>
        )}
      </div>
      {exp && (
        <div className="ib-bdesc">
          {d && <SplitNote d={d} />}
          <div className="ib-src">
            <Avatar actor={p.createdBy} className="ib-av" />
            <span>
              <b className={cx(p.createdBy.kind === 'agent' && 'mono')}>{actorName(p.createdBy)}</b> 记下 · {ago(p.createdAt, now)}
              {p.comments > 0 && <> · {p.comments} 条评论</>}
            </span>
            <span className="spacer" />
            <button className="btn sm ghost" type="button" onClick={() => openCard(p.id)} data-tip="动态、评论、附件" data-tip-pos="top">
              打开原条目
            </button>
          </div>
          <div className="desc-view">{p.body.trim() ? <Markdown src={p.body} repo={board?.repo} /> : <span className="ib-muted">没有描述</span>}</div>
        </div>
      )}
      <div className="ib-brows">
        {cards.map(c => (
          <Row key={renderKey(c.id)} card={c} now={now} labels={labels} all={all} home={home} checked={checked.includes(c.id)} open={open === c.id} onCheck={onCheck} inBundle />
        ))}
      </div>
    </div>
  );
}

const Row = memo(function Row({
  card: c, now, labels, all, home, checked, open, onCheck, inBundle,
}: { card: Card; now: number; labels: Label[]; all: Card[]; home: string; checked: boolean; open: boolean; onCheck: (id: number, shift: boolean) => void; inBundle?: boolean }) {
  const saving = useCardBusy(c.id);
  const flash = useUI(s => s.flashes[c.id]) as Flash | undefined;
  const pending = useData(s => s.pending[`${s.path}#${c.id}`]);
  const ref = useRef<HTMLDivElement>(null);
  const draft = isDraft(c.id);
  const isnew = useData(s => entryIsNew(s.path, c));
  useLayoutEffect(() => {
    if (flash) replay(ref.current, 'flash');
  }, [flash?.key]); // eslint-disable-line react-hooks/exhaustive-deps
  // 展开这一行就是人看过了：点去掉，结果同步到别的窗口和设备
  useLayoutEffect(() => {
    if (open && !draft) void markSeen(c.id);
  }, [open, draft, c.id]);
  const toggle = () => setInbox({ open: open ? null : c.id });
  const text = snippet(c.body);
  return (
    <div
      ref={ref}
      data-ib={renderKey(c.id)}
      className={cx('ib-row', checked && 'checked', open && 'open', saving && 'saving', draft && 'draft', isnew && 'is-new')}
      style={flash ? { ['--flash' as string]: flash.color } : undefined}
    >
      <div
        className="ib-line"
        role="button"
        tabIndex={0}
        aria-expanded={open}
        aria-label={`#${c.id} ${c.title}${isnew ? '（新的）' : ''}`}
        onClick={e => {
          if (draft) return;
          if (e.shiftKey) return onCheck(c.id, true);
          toggle();
        }}
        onKeyDown={e => {
          if (e.target !== e.currentTarget || draft) return;
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            toggle();
          }
          if (e.key === 'x') onCheck(c.id, e.shiftKey);
        }}
      >
        {draft ? <span className="ib-check" /> : <Check on={checked} label={`勾选 #${c.id}`} onToggle={shift => onCheck(c.id, shift)} />}
        <span className="ib-idwrap">
          {/* 新条目：编号前面一个点（展开看过就消失，PRD §10.5） */}
          {isnew && <i className="ib-newdot" aria-label="新的" role="img" />}
          <span className="ib-id mono">#{draft ? '…' : c.id}</span>
        </span>
        <Avatar actor={c.createdBy} className="ib-av" />
        <span className="ib-t">
          <b>
            {/* 手机上不显示编号那一栏：新的标记放在标题前面 */}
            {isnew && <i className="ib-newdot ib-newdot-m" aria-hidden="true" />}
            <Title text={c.title} />
          </b>
          {text && <span>{text}</span>}
        </span>
        <span className="ib-labels">
          <SplitMarks card={c} inBundle={inBundle} />
          <LabelChips ids={c.labels} labels={labels} max={2} pending={pending} />
        </span>
        <span className="ib-meta">{saving ? <span className="ib-saving" role="status" aria-label="保存中" /> : <Age card={c} now={now} />}</span>
        <span className="ib-acts">
          <button
            type="button"
            className="icon-btn sm"
            aria-label={`整理到「${home}」`}
            data-tip={`整理到「${home}」`}
            disabled={saving || draft}
            onClick={e => {
              e.stopPropagation();
              void doTriage([c.id]);
            }}
          >
            <Icon name="move" />
          </button>
          <button
            type="button"
            className="icon-btn sm ib-del"
            aria-label="删除"
            data-tip="删除"
            disabled={saving || draft}
            onClick={e => {
              e.stopPropagation();
              void doDelete([c.id], [c]);
            }}
          >
            <Icon name="trash" />
          </button>
        </span>
        <Icon name="chevDown" className="ib-chev" />
      </div>
      <div className="ib-exp">
        <div>{open && <RowEdit card={c} now={now} all={all} home={home} saving={saving} />}</div>
      </div>
    </div>
  );
});

/** 展开的编辑区：来源、标题、标签、描述（和卡片详情同一套编辑器），下面是这一条的操作 */
function RowEdit({ card: c, now, all, home, saving }: { card: Card; now: number; all: Card[]; home: string; saving: boolean }) {
  const [pick, setPick] = useState<DOMRect | null>(null);
  const d = useData(s => cardDispatch(s.ws, s.path, c.id));
  return (
    <div className="ib-edit">
      {d && <SplitNote d={d} />}
      <div className="ib-src">
        <Avatar actor={c.createdBy} className="ib-av" />
        <span>
          <b className={cx(c.createdBy.kind === 'agent' && 'mono')}>{actorName(c.createdBy)}</b> 记下 · {ago(c.createdAt, now)}
          {c.updatedAt - c.createdAt > 60e3 && <> · 最近更新 {ago(c.updatedAt, now)}</>}
          {c.comments > 0 && <> · {c.comments} 条评论</>}
          {c.splitFrom != null && <> · 拆自 #{c.splitFrom}</>}
        </span>
      </div>
      <TitleEditor card={c} locked={false} busy={saving} />
      <CardLabels card={c} saving={saving} />
      <div className="ib-desc">
        <DescEditor card={c} locked={false} busy={saving} />
      </div>
      <div className="ib-editacts">
        <button className="btn sm ghost ib-del" type="button" disabled={saving} onClick={() => void doDelete([c.id], [c])}>
          <Icon name="trash" />
          删除
        </button>
        <span className="spacer" />
        <button className="btn sm ghost" type="button" onClick={() => openCard(c.id)} data-tip="动态、评论、附件" data-tip-pos="top">
          打开详情
        </button>
        <button className="btn sm" type="button" disabled={saving || all.length < 2} onClick={e => setPick(e.currentTarget.getBoundingClientRect())}>
          <Icon name="merge" />
          并到…
        </button>
        <button
          className="btn sm"
          type="button"
          disabled={saving || !!d}
          data-tip={d ? '已经派出去了：在上面取消或停止' : '派给一台机器上的 agent 拆成几张卡，拆出的卡记到未整理，这一条并进拆出的那组里'}
          data-tip-pos="top"
          onClick={() => openDispatch(c.id)}
        >
          <Icon name="dispatch" />
          派给 agent 拆解
        </button>
        <button
          className="btn sm"
          type="button"
          aria-haspopup="menu"
          disabled={saving}
          onClick={e => {
            const cur = useUI.getState().ctx;
            if (cur?.kind === 'move' && cur.id === String(c.id)) return closeCtx();
            openCtx({ kind: 'move', id: String(c.id), x: 0, y: 0, anchor: e.currentTarget.getBoundingClientRect() });
          }}
        >
          <Icon name="move" />
          移动到
        </button>
        <button className="btn sm brand" type="button" disabled={saving} onClick={() => void doTriage([c.id])}>
          整理到「{home}」
        </button>
      </div>
      {pick && (
        <MergePick
          anchor={pick}
          self={c}
          all={all}
          onClose={() => setPick(null)}
          onPick={id => {
            setPick(null);
            openMerge([id, c.id], id);
          }}
        />
      )}
    </div>
  );
}

/** 「并到…」：列出别的条目（相似的排前面，可以筛选），选一条后打开合并预览 */
function MergePick({ anchor, self, all, onPick, onClose }: { anchor: DOMRect; self: Card; all: Card[]; onPick: (id: number) => void; onClose: () => void }) {
  const [q, setQ] = useState('');
  const [on, setOn] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const rel = useMemo(() => related(self, all), [self, all]);
  const w = q.trim().toLowerCase();
  const list = all
    .filter(c => c.id !== self.id && !isDraft(c.id) && (!w || `#${c.id} ${c.title}`.toLowerCase().includes(w)))
    .sort((a, b) => Number(rel.has(b.id)) - Number(rel.has(a.id)));
  useEffect(() => {
    const f = requestAnimationFrame(() => setOn(true));
    const down = (e: PointerEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      onClose();
    };
    document.addEventListener('pointerdown', down, true);
    document.addEventListener('keydown', key, true);
    return () => {
      cancelAnimationFrame(f);
      document.removeEventListener('pointerdown', down, true);
      document.removeEventListener('keydown', key, true);
    };
  }, [onClose]);
  const width = Math.min(340, window.innerWidth - 24);
  const left = Math.max(12, Math.min(window.innerWidth - width - 12, anchor.right - width));
  const below = anchor.bottom + 6 + 360 < window.innerHeight;
  return createPortal(
    <div
      ref={ref}
      className={cx('ctx ib-pick', on && 'on')}
      style={{ left, width, ...(below ? { top: anchor.bottom + 6 } : { bottom: window.innerHeight - anchor.top + 6 }) }}
      role="dialog"
      aria-label={`把 #${self.id} 并到哪一条`}
    >
      <div className="ctx-title">
        <Icon name="merge" />
        <b>把 #{self.id} 并到哪一条？</b>
      </div>
      <input className="input ib-pick-q" autoFocus placeholder="筛选" value={q} onChange={e => setQ(e.target.value)} />
      <div className="ib-pick-list">
        {list.map(c => (
          <button key={c.id} type="button" data-do="pick" onClick={() => onPick(c.id)}>
            <span className="ib-id mono">#{c.id}</span>
            <span className="ib-pick-t">{c.title}</span>
            {rel.has(c.id) && <span className="ib-likely">相似</span>}
          </button>
        ))}
        {!list.length && <p className="sort-hint">没有别的条目了</p>}
      </div>
    </div>,
    document.body,
  );
}

/* ------------------------------ 批量 ------------------------------ */

function BulkBar({ checked, cards, home }: { checked: number[]; cards: Card[]; home: string }) {
  const cur = useCurrent();
  const n = checked.length;
  const bulkLabels = (rect: DOMRect) => {
    if (!cur) return;
    const ids = checked;
    const all = useData.getState().board?.cards ?? {};
    const common = (cur.project.labels ?? []).filter(l => ids.every(id => all[id]?.labels.includes(l.id))).map(l => l.id);
    openPicker({
      rect,
      project: cur.project.key,
      selected: common,
      onToggle: async (lid, on) => {
        const results = await Promise.allSettled(ids.map(id => (on ? addLabel(id, lid) : removeLabel(id, lid))));
        const bad = results.find(r => r.status === 'rejected') as PromiseRejectedResult | undefined;
        if (bad) toastError(bad.reason);
      },
    });
  };
  return (
    <div className={cx('ib-bulk', n > 0 && 'on')} role="toolbar" aria-label="批量操作" aria-hidden={!n} inert={!n || undefined}>
      <span className="ib-bulk-n">
        <b className="mono" key={n}>
          {n}
        </b>
        条已选
      </span>
      <i className="ib-bulk-sep" />
      <button type="button" className="btn sm ghost" disabled={n < 2} data-tip={n < 2 ? '至少勾选两条' : undefined} data-tip-pos="top" onClick={() => openMerge(checked)}>
        <Icon name="merge" />
        <span className="ib-bl">合并成一条</span>
      </button>
      <button type="button" className="btn sm ghost" data-label-picker onClick={e => bulkLabels(e.currentTarget.getBoundingClientRect())}>
        <Icon name="tag" />
        <span className="ib-bl">标签</span>
      </button>
      <button type="button" className="btn sm ghost ib-del" onClick={() => void doDelete(checked, cards)}>
        <Icon name="trash" />
        <span className="ib-bl">删除</span>
      </button>
      <button
        type="button"
        className="btn sm brand"
        onClick={() => {
          const ids = checked;
          setInbox({ checked: [] });
          void doTriage(ids);
        }}
      >
        整理到「{home}」
      </button>
      <button type="button" className="icon-btn sm" aria-label="取消选择" data-tip="取消选择 · esc" data-tip-pos="top" onClick={() => setInbox({ checked: [] })}>
        <Icon name="close" />
      </button>
    </div>
  );
}

/* ------------------------------ 合并预览 ------------------------------ */

function MergeSheet({ req }: { req: MergeReq }) {
  const board = useData(s => s.board);
  const cur = useCurrent();
  const now = useNow(60000);
  const src = req.ids.map(id => board?.cards[id]).filter(Boolean) as Card[];
  const target = src.find(c => c.id === req.keep);
  const others = src.filter(c => c.id !== req.keep);
  const auto = target ? { title: target.title, body: mergedBody(target, others) } : { title: '', body: '' };
  const v = req.draft ?? auto;
  const labels = [...new Set(src.flatMap(c => c.labels))];
  const [on, setOn] = useState(false);
  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const ta = useRef<HTMLTextAreaElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => autosize(ta.current), [v.body, mode]);
  const setDraft = (d: { title: string; body: string } | null) => setInbox({ merge: { ...req, draft: d } });
  const close = () => {
    setOn(false);
    setTimeout(() => setInbox({ merge: null }), 260);
  };
  useEffect(() => {
    const f = requestAnimationFrame(() => setOn(true));
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || useUI.getState().picker) return;
      e.stopPropagation();
      close();
    };
    document.addEventListener('keydown', key, true);
    return () => {
      cancelAnimationFrame(f);
      document.removeEventListener('keydown', key, true);
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  // 涉及的条目被别处删掉、整理走了：少于两条就没法合并了
  useEffect(() => {
    if (board && src.length < 2) close();
  }, [board, src.length]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!target) return null;
  const busy = src.some(c => cardBusy(c.id));
  const submit = () => {
    const title = v.title.trim();
    if (!title) {
      replay(titleRef.current, 'invalid');
      titleRef.current?.focus();
      return;
    }
    setOn(false);
    void doMerge(req, { title, body: v.body.trim() });
  };
  return createPortal(
    <>
      <div className={cx('scrim', on && 'on')} onClick={close} />
      <div className={cx('drawer ib-msheet', on && 'on')} role="dialog" aria-modal="true" aria-label={`合并 ${src.length} 条`}>
        <div className="d-head">
          <Icon name="merge" />
          <b>合并 {src.length} 条</b>
          <span className="ib-muted">合并后只留一条，其余几条的内容接进描述里</span>
          <span className="spacer" />
          <button className="icon-btn" type="button" aria-label="关闭" data-tip="关闭  esc" data-tip-align="end" onClick={close}>
            <Icon name="close" />
          </button>
        </div>
        <div className="d-grid">
          <div className="d-main">
            <section className="d-section" style={{ ['--i' as string]: 0 }}>
              <div className="d-label">合并后的标题</div>
              <input
                ref={titleRef}
                className="input ib-msheet-title"
                value={v.title}
                maxLength={200}
                onChange={e => setDraft({ ...v, title: e.target.value })}
                onKeyDown={e => {
                  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) submit();
                }}
              />
              {labels.length > 0 && (
                <div className="d-labels">
                  {(cur?.project.labels ?? [])
                    .filter(l => labels.includes(l.id))
                    .map(l => (
                      <LabelChip key={l.id} label={l} />
                    ))}
                </div>
              )}
            </section>
            <section className="d-section" style={{ ['--i' as string]: 1 }}>
              <div className="d-label">
                合并后的描述
                {req.draft && (
                  <button type="button" className="ib-link" onClick={() => setDraft(null)}>
                    恢复自动拼好的
                  </button>
                )}
                <span className="act">
                  <DescTabs
                    mode={mode}
                    setMode={m => setMode(m as 'view' | 'edit')}
                    modes={[
                      ['view', '预览'],
                      ['edit', '编辑'],
                    ]}
                  />
                </span>
              </div>
              {mode === 'edit' ? (
                <textarea
                  ref={ta}
                  className="textarea ib-msheet-body"
                  spellCheck={false}
                  value={v.body}
                  onChange={e => setDraft({ ...v, body: e.target.value })}
                  onKeyDown={e => {
                    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) submit();
                  }}
                />
              ) : (
                <div className="desc-view">{v.body.trim() ? <Markdown src={v.body} repo={board?.repo} /> : <span className="ib-muted">没有描述</span>}</div>
              )}
            </section>
          </div>
          <div className="d-side ib-msheet-side">
            <div className="d-label d-side-head">保留哪一条的编号</div>
            <div className="ib-keep" role="radiogroup" aria-label="保留哪一条的编号">
              {src.map(c => (
                <label key={c.id} className={cx('ib-keep-row', req.keep === c.id && 'on')}>
                  <input type="radio" name="ib-keep" checked={req.keep === c.id} onChange={() => setInbox({ merge: { ...req, keep: c.id, draft: null } })} />
                  <i />
                  <span className="ib-keep-main">
                    <b>
                      <span className="ib-id mono">#{c.id}</span> <Title text={c.title} />
                    </b>
                    <span>
                      <Avatar actor={c.createdBy} className="ib-av xs" />
                      {actorName(c.createdBy)} · {ago(c.createdAt, now)}
                    </span>
                  </span>
                </label>
              ))}
            </div>
            <p className="ib-keep-hint">标签合并成并集；其余几条的附件、评论归到留下的这条，然后从「未整理」里删掉。</p>
          </div>
        </div>
        <div className="ib-msheet-foot">
          <button className="btn sm ghost" type="button" onClick={close}>
            取消
          </button>
          <button className="btn sm brand" type="button" disabled={busy} onClick={submit}>
            <Icon name="merge" />
            合并成 #{req.keep}
          </button>
        </div>
      </div>
    </>,
    document.body,
  );
}

/* ------------------------------ 侧栏 ------------------------------ */

/** 侧栏里「未整理」的来源和时间段：点来源只看它（再点取消），点时间段跳到那一组 */
export function InboxSideParts() {
  const { all } = useInboxCards();
  const src = useInbox(s => s.src);
  const now = useNow(60000);
  const sources = useMemo(() => {
    const m = new Map<string, { actor: Actor; n: number }>();
    for (const c of all ?? []) {
      const k = srcKey(c.createdBy);
      m.set(k, { actor: c.createdBy, n: (m.get(k)?.n ?? 0) + 1 });
    }
    return [...m.entries()].sort(([a, x], [b, y]) => Number(a.startsWith('human')) - Number(b.startsWith('human')) || y.n - x.n);
  }, [all]);
  const counts = BUCKETS.map(k => ({ k, n: (all ?? []).filter(c => bucketOf(c.createdAt, now) === k).length }));
  if (!all) return <SideSkeleton />;
  return (
    <>
      {sources.length > 0 && (
        <SideSec title="来源">
          {sources.map(([k, { actor, n }]) => (
            <button key={k} type="button" className={cx('ib-side-row', src === k && 'on', src && src !== k && 'dim')} aria-pressed={src === k} onClick={() => setInbox({ src: src === k ? null : k })}>
              <Avatar actor={actor} className="ib-av" />
              <span className={cx('ib-side-name', actor.kind === 'agent' && 'mono')}>{actorName(actor)}</span>
              <b className="mono">{n}</b>
            </button>
          ))}
        </SideSec>
      )}
      <SideSec title="放了多久">
        {counts.map(({ k, n }) => (
          <button
            key={k}
            type="button"
            className={cx('ib-side-row', k === 'stale' && 'stale')}
            disabled={!n}
            onClick={() => {
              setGroup('time');
              setTimeout(() => document.querySelector(`.page-inbox [data-group="${k}"]`)?.scrollIntoView({ behavior: reduceMotion() ? 'auto' : 'smooth', block: 'start' }), 40);
            }}
          >
            {k === 'stale' ? <Icon name="timer" /> : <i className="ib-side-dot" />}
            <span className="ib-side-name">{BUCKET_NAME[k]}</span>
            <b className="mono">{n}</b>
          </button>
        ))}
      </SideSec>
      <p className="ib-side-hint">点一行展开修改；勾选几条一起合并、打标签或整理。按住 Shift 连着勾，{isMac ? '⌘' : 'Ctrl+'}A 全选。</p>
    </>
  );
}

function SideSec({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="ib-side-sec">
      <div className="ib-side-h">{title}</div>
      {children}
    </div>
  );
}

function SideSkeleton() {
  return (
    <div className="ib-side-sec" aria-busy="true">
      {[70, 52, 60].map((w, i) => (
        <div key={i} className="ib-side-row sk">
          <Skeleton w={20} h={20} r={999} />
          <Skeleton w={`${w}%`} h={10} />
        </div>
      ))}
    </div>
  );
}

/* ------------------------------ 动效 ------------------------------ */

const SPRING = 'cubic-bezier(.22, 1.18, .36, 1)';

/** 行的进出与补位（自动 FLIP）：每次布局后记下各行的位置（相对列表，滚动不算移动）。
    行的集合或顺序变了时：留下的行从旧位置弹到新位置；新出现的行（agent 刚记下、撤销回来的）淡入；
    真的离开「未整理」的行（整理、删除、合并）在原位播离场（exits 里记着是哪一种）。
    换白板时整体重建，不补间；只是被筛选掉的行直接消失 */
function useRowMotion(root: RefObject<HTMLElement | null>, present: Set<number>, path: string) {
  const last = useRef<{ path: string; sig: string; rows: Map<number, { el: HTMLElement; top: number; rect: DOMRect }> } | null>(null);
  useLayoutEffect(() => {
    const r = root.current;
    if (!r) return;
    const base = r.getBoundingClientRect().top;
    const els = [...r.querySelectorAll<HTMLElement>('[data-ib]')];
    const rows = new Map(
      els.map(el => {
        const rect = el.getBoundingClientRect();
        return [Number(el.dataset.ib), { el, top: rect.top - base, rect }] as const;
      }),
    );
    const sig = els.map(el => el.dataset.ib).join(',');
    const prev = last.current;
    last.current = { path, sig, rows };
    if (!prev || prev.path !== path || prev.sig === sig || reduceMotion()) return;
    for (const [id, row] of rows) {
      const p = prev.rows.get(id);
      if (!p) {
        if (present.has(id) && !prev.rows.size) continue;
        row.el.animate(
          [
            { opacity: 0, transform: 'translateY(-6px)', backgroundColor: 'color-mix(in srgb, var(--c-inbox) 16%, transparent)' },
            { opacity: 1, transform: 'none', offset: 0.4 },
            { backgroundColor: 'transparent' },
          ],
          { duration: 900, easing: 'cubic-bezier(.16, 1, .3, 1)' },
        );
        continue;
      }
      const dy = p.top - row.top;
      if (Math.abs(dy) > 1) row.el.animate([{ transform: `translateY(${dy}px)` }, { transform: 'none' }], { duration: 560, easing: SPRING });
    }
    for (const [id, p] of prev.rows) {
      if (rows.has(id) || present.has(id) || !p.rect.width || p.el.isConnected) continue;
      ghost(p.el, p.rect, exits.get(id));
      exits.delete(id);
    }
  });
}

function ghost(el: HTMLElement, r: DOMRect, kind: 'triage' | 'delete' | 'merge' | undefined) {
  if (r.bottom < 0 || r.top > window.innerHeight) return;
  Object.assign(el.style, {
    position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, margin: '0', zIndex: '60', pointerEvents: 'none',
  });
  el.classList.add('ib-ghost');
  document.body.appendChild(el);
  const to =
    kind === 'triage'
      ? { opacity: 0, transform: 'translateX(56px)' }
      : kind === 'merge'
        ? { opacity: 0, transform: 'translateY(-10px) scale(.94)' }
        : { opacity: 0, transform: 'scale(.97)', filter: 'blur(4px)' };
  el.animate([{ opacity: 1, transform: 'none', filter: 'blur(0)' }, to], { duration: 380, easing: 'cubic-bezier(.4, 0, .2, 1)', fill: 'forwards' }).onfinish = () => el.remove();
}
