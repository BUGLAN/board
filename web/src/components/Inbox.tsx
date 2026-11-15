/* ==========================================================================
   「未整理」页（PRD §10.5）：当前白板「未整理」里的全部条目摊开在主区，方便改、合并、整理
   - 按时间 / 来源 / 标签分组，一行一条；点一行原地展开编辑（标题、标签、描述，和卡片详情同一套编辑器）
   - 勾选（Shift 连选）后底部浮出批量条：合并成一条、标签、删除、整理到首个可领取状态
   - 合并先预览：选保留哪一条的编号，合并后的标题和描述可以先改再确认
   - 两种视图（default/main#286）：列表（一行一条）和逐条（一次一条的完整内容，左右按钮、← → 键切换，在这一条上直接处理；
     整理走、删掉后自动到下一条），切换入口和分组放在一起，记住上次用的
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
import { judge } from '../lib/granularity';
import { cardDispatch, hostWords } from '../lib/hosts';
import { boardFlow, INBOX } from '../lib/schema';
import { canUndoSplit, kidsOf, hiddenEntries, laterEntries, laterGroups, unitCards, unitRep, unitsOf, type Unit } from '../lib/split';
import { swipeCommits, swipeOffset, type SwipeDir } from '../lib/swipe';
import { deleteCards, undoDelete } from '../lib/trash';
import type { Actor, Card, Label } from '../lib/types';
import { actorName, ago, autosize, cx, pref, reduceMotion, replay } from '../lib/util';
import { addLabel, cardBusy, cardKey, entryIsNew, exec, isDraft, markAllSeen, markSeen, mergePatch, movePatch, newEntryCount, removeLabel, renderKey, serverNow, setLater, useCardBusy, useCurrent, useData } from '../store/data';
import { confirm, openCreator, openCtx, closeCtx, openDispatch, openPicker, openSession, overlayOpen, reveal, setFilter, toast, toastError, useUI, type Flash } from '../store/ui';
import { ActorAvatar, DescTabs, LabelChip, LabelChips, Markdown, Tip, Title, useNow } from './bits';
import { matches } from './Card';
import { CardLabels, DescEditor, TitleEditor } from './CardDetail';
import { Icon } from './Icon';
import { Skeleton } from './Skeleton';
import { cancel } from './hosts/Dispatch';

/* ------------------------------ 状态 ------------------------------ */

type Group = 'time' | 'source' | 'label';
type View = 'list' | 'one';
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
  view: View;
  /** 逐条视图里显示的那一条 */
  focus: number | null;
  /** 看「稍后再做」里收起的条目（default/main#303） */
  later: boolean;
}
const GROUPS: Group[] = ['time', 'source', 'label'];
const savedGroup = pref.get<Group>('inboxGroup', 'time');
const savedView = pref.get<View>('inboxView', 'list');
export const useInbox = create<InboxUI>(() => ({
  src: null, group: GROUPS.includes(savedGroup) ? savedGroup : 'time', checked: [], open: null, merge: null,
  view: savedView === 'one' ? 'one' : 'list', focus: null, later: false,
}));
const setInbox = (p: Partial<InboxUI>) => useInbox.setState(p);
const setGroup = (group: Group) => {
  setInbox({ group });
  pref.set('inboxGroup', group);
};
const setView = (view: View) => {
  // 逐条视图里没有勾选：切过去时清掉，免得批量条挂在那儿
  setInbox(view === 'one' ? { view, checked: [], open: null } : { view });
  pref.set('inboxView', view);
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
      .filter(c => c && !c.laterAt)
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

/** 行离开时在原位播的动画：整理、收到「稍后再做」→ 往右滑走，放回「未整理」→ 往左滑走，删除 → 原地淡出，合并 → 往上收 */
/** gone：行已经在列表里原地收起了（横滑走的、行上按钮收起 / 放回的），移走时不再播离场 */
type Exit = 'triage' | 'delete' | 'merge' | 'later' | 'back' | 'gone';
const exits = new Map<number, Exit>();

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

/** 收到「稍后再做」（default/main#303）：当帧从列表里消失，提示里可以「撤销」；失败的回到原处并提示 */
async function doLater(ids: number[], gone = false) {
  const ok = ids.filter(id => !isDraft(id));
  if (!ok.length) return;
  const st = useInbox.getState();
  setInbox({ checked: st.checked.filter(c => !ok.includes(c)), open: st.open != null && ok.includes(st.open) ? null : st.open });
  ok.forEach(id => exits.set(id, gone ? 'gone' : 'later'));
  const done = setLater(ok, true);
  toast({
    type: 'success',
    title: ok.length === 1 ? `#${ok[0]} 已移到「稍后再做」` : `${ok.length} 条已移到「稍后再做」`,
    text: '在标题旁的「稍后再做」里查看，左滑放回',
    action: { label: '撤销', fn: () => void setLater(ok, false).then(errs => errs.length && toastError(errs[0])) },
  });
  const errs = await done;
  if (errs.length) toastError(errs[0]);
}

/** 从「稍后再做」放回「未整理」：回到原来的位置，标签、描述、评论都不动；当帧离开这一页，提示里可以「撤销」 */
async function doUnlater(ids: number[], gone = false) {
  const ok = ids.filter(id => !isDraft(id));
  if (!ok.length) return;
  ok.forEach(id => exits.set(id, gone ? 'gone' : 'back'));
  const done = setLater(ok, false);
  toast({
    type: 'success',
    title: ok.length === 1 ? `#${ok[0]} 已放回「未整理」` : `${ok.length} 条已放回「未整理」`,
    action: { label: '撤销', fn: () => void setLater(ok, true).then(errs => errs.length && toastError(errs[0])) },
  });
  const errs = await done;
  if (errs.length) toastError(errs[0]);
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
  const view = useInbox(s => s.view);
  const checked = useInbox(s => s.checked);
  const open = useInbox(s => s.open);
  const merge = useInbox(s => s.merge);
  const later = useInbox(s => s.later);
  const laterCards = useMemo(() => (board ? laterEntries(board) : null), [board]);
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
  useEffect(() => setInbox({ checked: [], open: null, src: null, merge: null, focus: null, later: false }), [path]);
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
      if (st.view === 'one' || st.later) return;
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
              <TitleTabs later={later} count={count} laterN={laterCards?.length ?? null} />
              {/* agent 新记下、人还没看过的条数；没有时不占地方 */}
              {!later && all && newCount > 0 && (
                <div className="ib-newline">
                  <span className="ib-new mono">{newCount} 条新的</span>
                  <button type="button" className="ib-link ib-markall" onClick={() => void doMarkAll(setSent)}>
                    全部标为已读
                  </button>
                </div>
              )}
            </div>
            {/* 列表 / 逐条、分组只管「未整理」：看「稍后再做」时不放 */}
            {!later && <div className="ib-modes">
              <DescTabs
                mode={view}
                setMode={m => setView(m as View)}
                modes={[
                  ['list', '列表'],
                  ['one', '逐条'],
                ]}
              />
              <DescTabs
                mode={group}
                setMode={m => setGroup(m as Group)}
                modes={[
                  ['time', '按时间'],
                  ['source', '按来源'],
                  ['label', '按标签'],
                ]}
              />
            </div>}
          </header>
          {later && <p className="ib-later-sub">暂时不处理的条目收在这里，不算在「未整理」的条数里。左滑一条放回「未整理」，回到原来的位置。</p>}

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
          {!later && !list && (cur?.board.counts[INBOX] ?? 3) > 0 && <RowsSkeleton n={Math.min(cur?.board.counts[INBOX] ?? 3, 5)} />}
          {later && <LaterPage cards={laterCards} labels={labels} project={cur?.project.key ?? ''} path={path} />}
          {!later && view === 'one' && list && list.length > 0 && <OneView groups={groups} now={now} all={all ?? []} home={home} />}
          {!later && view === 'list' && groups.map(g => (
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
          {!later && list && !list.length && (
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
          {!later && (filter || src) && list && list.length > 0 && (
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
      {!later && view === 'list' && <BulkBar checked={checked} cards={all ?? []} home={home} />}
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

/** 勾选框：自己画的方块（选中打勾、部分选中一道横线），点的时候弹一下。
    一个按钮就是一个勾选框（读屏报出名称和勾没勾），空格、回车切换，按住 Shift 连选；
    「全选」和各行的勾选一起按 aria-checked / aria-label 报出来（default/main#315） */
export function Check({ on, some, label, onToggle }: { on: boolean; some?: boolean; label: string; onToggle: (shift: boolean) => void }) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={some ? 'mixed' : on}
      aria-label={label}
      className={cx('ib-check', on && 'on', some && 'some')}
      onClick={e => {
        e.stopPropagation();
        onToggle(e.shiftKey);
      }}
      onKeyDown={e => {
        // 回车照常触发 click；空格默认会滚页面，这里自己切换
        if (e.key !== ' ') return;
        e.preventDefault();
        e.stopPropagation();
        onToggle(e.shiftKey);
      }}
    >
      <i />
    </button>
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
        <b>{d.state === 'queued' ? '排队拆解' : '拆解中'}</b> · {hostWords(d)} · {d.agentLabel}
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
    <Tip text={d.state === 'queued' ? '排队等机器有空位' : `${d.name ?? ''} 正在拆这一条`}>
      <span className={cx('ib-mark ib-splitting', d.state === 'queued' && 'queued')}>
        <i aria-hidden="true" />
        {d.state === 'queued' ? '排队拆解' : '拆解中'} · {hostWords(d)} · {d.agentLabel}
      </span>
    </Tip>
  );
}

/** 这一行的拆解标记：拆解中（排队拆解）· 机器 · agent；拆自 #ID（在拆解组里时组头已经写了，不再标） */
function SplitMarks({ card: c, inBundle }: { card: Card; inBundle?: boolean }) {
  const d = useData(s => cardDispatch(s.ws, s.path, c.id));
  return (
    <>
      {d && <SplittingMark d={d} />}
      {c.splitFrom != null && !inBundle && (
        <Tip text={`从 #${c.splitFrom} 拆出来的`}>
          <span className="ib-mark ib-from">
            拆自 #{c.splitFrom}
          </span>
        </Tip>
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
          <Tip text={d ? '还在拆：先停止' : '删掉拆出的卡（进回收站），原条目回到未整理'}>
            <button
              type="button"
              className="btn sm ghost ib-undo-split"
              disabled={busy || !!d}
              data-tip-pos="top"
              onClick={() => void undoSplit(p)}
            >
              <Icon name="replay" />
              撤销拆解
            </button>
          </Tip>
        )}
      </div>
      {exp && (
        <div className="ib-bdesc">
          {d && <SplitNote d={d} />}
          <div className="ib-src">
            <ActorAvatar actor={p.createdBy} className="ib-av" />
            <span>
              <b className={cx(p.createdBy.kind === 'agent' && 'mono')}>{actorName(p.createdBy)}</b> 记下 · {ago(p.createdAt, now)}
              {p.comments > 0 && <> · {p.comments} 条评论</>}
            </span>
            <span className="spacer" />
            <Tip text="动态、评论、附件">
              <button className="btn sm ghost" type="button" onClick={() => openCard(p.id)} data-tip-pos="top">
                打开原条目
              </button>
            </Tip>
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
  // 往右滑：直接收到「稍后再做」（default/main#303）
  const sw = useSwipe(1, !draft && !saving, () => void doLater([c.id], true));
  return (
    <div
      ref={ref}
      data-ib={renderKey(c.id)}
      className={cx('ib-row', checked && 'checked', open && 'open', saving && 'saving', draft && 'draft', isnew && 'is-new', sw.state && `swipe-${sw.state}`)}
      style={flash ? { ['--flash' as string]: flash.color } : undefined}
    >
      <div className="ib-swipe">
      <SwipeHint dir={1} label="稍后再做" icon="timer" />
      <div
        className="ib-line"
        role="button"
        tabIndex={0}
        aria-expanded={open}
        aria-label={`#${c.id} ${c.title}${isnew ? '（新的）' : ''}`}
        style={sw.style}
        {...sw.handlers}
        onClick={e => {
          if (draft || sw.swallow()) return;
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
        {draft ? <span className="ib-check" aria-hidden="true" /> : <Check on={checked} label={`勾选 #${c.id} ${c.title}`} onToggle={shift => onCheck(c.id, shift)} />}
        <span className="ib-idwrap">
          {/* 新条目：编号前面一个点（展开看过就消失，PRD §10.5） */}
          {isnew && <i className="ib-newdot" aria-label="新的" role="img" />}
          <span className="ib-id mono">#{draft ? '…' : c.id}</span>
        </span>
        <ActorAvatar actor={c.createdBy} className="ib-av" />
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
          <Tip text={`整理到「${home}」`}>
            <button
              type="button"
              className="icon-btn sm"
              aria-label={`整理到「${home}」`}
              disabled={saving || draft}
              onClick={e => {
                e.stopPropagation();
                void doTriage([c.id]);
              }}
            >
              <Icon name="move" />
            </button>
          </Tip>
          <Tip text="稍后再做：先收起来，不在列表里">
            <button
              type="button"
              className="icon-btn sm"
              aria-label="稍后再做"
              disabled={saving || draft}
              onClick={e => {
                e.stopPropagation();
                // 和右滑一样：行往右滑出去、原地收起，再移走
                sw.leave(e.currentTarget);
              }}
            >
              <Icon name="timer" />
            </button>
          </Tip>
          <Tip text="删除">
            <button
              type="button"
              className="icon-btn sm ib-del"
              aria-label="删除"
              disabled={saving || draft}
              onClick={e => {
                e.stopPropagation();
                void doDelete([c.id], [c]);
              }}
            >
              <Icon name="trash" />
            </button>
          </Tip>
        </span>
        <Icon name="chevDown" className="ib-chev" />
      </div>
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
  // 粒度预检（default/main#306）：已经拆出来的子项不再判；偏小的不拆，要拆需点「强制拆解」
  const verdict = c.splitFrom == null ? judge(c.title, c.body) : null;
  const small = verdict?.size === 'small' ? verdict : null;
  return (
    <div className="ib-edit">
      {d && <SplitNote d={d} />}
      <div className="ib-src">
        <ActorAvatar actor={c.createdBy} className="ib-av" />
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
        <button className="btn sm ghost" type="button" disabled={saving} onClick={() => void doLater([c.id])}>
          <Icon name="timer" />
          稍后再做
        </button>
        <span className="spacer" />
        <Tip text="动态、评论、附件">
          <button className="btn sm ghost" type="button" onClick={() => openCard(c.id)} data-tip-pos="top">
            打开详情
          </button>
        </Tip>
        <button className="btn sm" type="button" disabled={saving || all.length < 2} onClick={e => setPick(e.currentTarget.getBoundingClientRect())}>
          <Icon name="merge" />
          并到…
        </button>
        <Tip
          text={
            d
              ? '已经派出去了：在上面取消或停止'
              : small
                ? `任务粒度合适，不用拆（${small.why}）；点「强制拆解」仍可派给 agent 再切`
                : '派给一台机器上的 agent 拆成几张卡，拆出的卡记到未整理，这一条并进拆出的那组里'
          }
        >
          <button
            className="btn sm"
            type="button"
            disabled={saving || !!d}
            data-tip-pos="top"
            onClick={() => openDispatch(c.id, { force: !!small })}
          >
            <Icon name="dispatch" />
            {small ? '强制拆解' : '派给 agent 拆解'}
          </button>
        </Tip>
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

/* ------------------------------ 逐条视图 ------------------------------ */

/** 输入框、编辑器里按方向键是移动光标，不切换 */
const typing = (t: EventTarget | null) => !!(t as HTMLElement | null)?.closest?.('input, textarea, select, [contenteditable]');

/** 逐条：一次显示一条的完整内容，左右按钮、← → 键切换上一条 / 下一条（顺序和列表视图一样）。
    显示的这一条整理走、删掉、并走之后，自动显示原来位置上的下一条（到底了就是上一条） */
function OneView({ groups, now, all, home }: { groups: GroupOf[]; now: number; all: Card[]; home: string }) {
  const flat = useMemo(() => groups.flatMap(g => g.rows.map(c => ({ c, g: g.label }))).filter(x => !isDraft(x.c.id)), [groups]);
  const focus = useInbox(s => s.focus);
  const last = useRef(0);
  const dir = useRef<1 | -1>(1);
  const found = flat.findIndex(x => x.c.id === focus);
  const i = found >= 0 ? found : Math.min(last.current, flat.length - 1);
  const cur = flat[i];
  const id = cur?.c.id ?? 0;
  const saving = useCardBusy(id);
  useEffect(() => {
    if (found >= 0) last.current = found;
    else if (cur) setInbox({ focus: cur.c.id });
  }, [found, cur]);
  // 看到了就是人看过了：去掉「新」的点（同列表里展开一行）
  useEffect(() => {
    if (id > 0) void markSeen(id);
  }, [id]);
  const go = (d: 1 | -1) => {
    const n = flat[i + d];
    if (!n) return;
    dir.current = d;
    setInbox({ focus: n.c.id });
  };
  const goRef = useRef(go);
  goRef.current = go;
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
      if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target) || overlayOpen() || useData.getState().detailId != null || useInbox.getState().merge) return;
      e.preventDefault();
      goRef.current(e.key === 'ArrowLeft' ? -1 : 1);
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, []);
  if (!cur) return null;
  const c = cur.c;
  return (
    <div className="ib-one">
      <div className="ib-one-nav">
        <Tip text="上一条  ←">
          <button type="button" className="icon-btn" aria-label="上一条" disabled={i <= 0} onClick={() => go(-1)}>
            <Icon name="chevLeft" />
          </button>
        </Tip>
        <span className="ib-one-pos">
          <b className="mono">{i + 1}</b>
          <span className="mono"> / {flat.length}</span>
          <span className="ib-one-grp">{cur.g}</span>
        </span>
        <Tip text="下一条  →">
          <button type="button" className="icon-btn" aria-label="下一条" data-tip-align="end" disabled={i >= flat.length - 1} onClick={() => go(1)}>
            <Icon name="chevRight" />
          </button>
        </Tip>
      </div>
      <article key={c.id} className={cx('ib-one-card', dir.current > 0 ? 'from-r' : 'from-l', saving && 'saving')} aria-label={`#${c.id} ${c.title}`}>
        <div className="ib-one-head">
          <span className="ib-id mono">#{c.id}</span>
          {/* 拆自、记下多久在下面来源那一行里写了：这里只标拆解中 */}
          <SplitMarks card={c} inBundle />
          <span className="spacer" />
          {saving && <span className="ib-saving" role="status" aria-label="保存中" />}
        </div>
        <RowEdit card={c} now={now} all={all} home={home} saving={saving} />
      </article>
    </div>
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
      <Tip text={n < 2 ? '至少勾选两条' : ''}>
        <button type="button" className="btn sm ghost" disabled={n < 2} data-tip-pos="top" onClick={() => openMerge(checked)}>
          <Icon name="merge" />
          <span className="ib-bl">合并成一条</span>
        </button>
      </Tip>
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
      <Tip text="取消选择 · esc">
        <button type="button" className="icon-btn sm" aria-label="取消选择" data-tip-pos="top" onClick={() => setInbox({ checked: [] })}>
          <Icon name="close" />
        </button>
      </Tip>
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
          <Tip text="关闭  esc">
            <button className="icon-btn" type="button" aria-label="关闭" data-tip-align="end" onClick={close}>
              <Icon name="close" />
            </button>
          </Tip>
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
                      <ActorAvatar actor={c.createdBy} className="ib-av xs" />
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
              <ActorAvatar actor={actor} className="ib-av" />
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
function useRowMotion(root: RefObject<HTMLElement | null>, present: Set<number>, path: string, attr: 'ib' | 'lt' = 'ib') {
  const last = useRef<{ path: string; sig: string; rows: Map<number, Spot> } | null>(null);
  useLayoutEffect(() => {
    const r = root.current;
    if (!r) return;
    const box = r.getBoundingClientRect();
    const base = box.top;
    const els = [...r.querySelectorAll<HTMLElement>(`[data-${attr}]`)];
    const rows = new Map(
      els.map(el => {
        const rect = el.getBoundingClientRect();
        // 拆解组里的行不在整块列表的外角上
        const inner = !!el.parentElement?.classList.contains('ib-brows');
        const spot: Spot = { el, top: rect.top - base, left: rect.left - box.left, rect, edge: { top: !inner && !el.previousElementSibling, bottom: !inner && !el.nextElementSibling } };
        return [Number(el.dataset[attr]), spot] as const;
      }),
    );
    const sig = els.map(el => el.dataset[attr]).join(',');
    const prev = last.current;
    last.current = { path, sig, rows };
    if (!prev || prev.path !== path || prev.sig === sig || reduceMotion()) return;
    // 这次移走的行已经在列表里原地收起过（横滑走、行上按钮收起 / 放回）：下面的行跟着收起已经到了新位置，
    // 记下的旧位置是收起之前的，再补位会先往下弹一下再回来，所以这一轮不补位
    const collapsed = [...prev.rows.keys()].some(id => !rows.has(id) && exits.get(id) === 'gone');
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
      if (!collapsed && Math.abs(dy) > 1) row.el.animate([{ transform: `translateY(${dy}px)` }, { transform: 'none' }], { duration: 560, easing: SPRING });
    }
    for (const [id, p] of prev.rows) {
      if (rows.has(id) || present.has(id) || !p.rect.width || p.el.isConnected) continue;
      if (exits.get(id) === 'gone') {
        exits.delete(id);
        continue;
      }
      // 位置按列表现在的位置算（记下位置之后页面可能滚动过）
      const now = r.getBoundingClientRect();
      ghost(p.el, new DOMRect(now.left + p.left, now.top + p.top, p.rect.width, p.rect.height), exits.get(id), p.edge);
      exits.delete(id);
    }
  });
}

/** 一行在列表里的位置：相对列表的偏移、上次的矩形、是不是整块列表的上 / 下外角 */
type Spot = { el: HTMLElement; top: number; left: number; rect: DOMRect; edge: { top: boolean; bottom: boolean } };

function ghost(el: HTMLElement, r: DOMRect, kind: Exit | undefined, edge: Spot['edge']) {
  if (r.bottom < 0 || r.top > window.innerHeight) return;
  // 横滑走的（收起、放回）还是列表里的那一行：圆角按它在列表里的位置（第一行只圆上角，中间的行全是直角），不另加描边
  const flat = kind === 'later' || kind === 'back';
  const rad = (on: boolean) => (on ? 'var(--r-md)' : '0');
  Object.assign(el.style, {
    position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, margin: '0', zIndex: '60', pointerEvents: 'none',
    ...(flat ? { borderRadius: `${rad(edge.top)} ${rad(edge.top)} ${rad(edge.bottom)} ${rad(edge.bottom)}` } : {}),
  });
  el.classList.add('ib-ghost');
  if (flat) el.classList.add('flat');
  document.body.appendChild(el);
  if (flat) {
    // 行已经滑出去了，剩下底下的提示色块：原地收起高度（和下面补位的行同一条曲线、同样长，补位的行正好贴着它的下沿），
    // 同时淡出；不浮在列表上、不横移出列表
    el.style.overflow = 'hidden';
    el.animate([{ height: `${r.height}px` }, { height: '0px' }], { duration: 560, easing: SPRING, fill: 'forwards' }).onfinish = () => el.remove();
    el.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 320, easing: 'ease-out', fill: 'forwards' });
    return;
  }
  const to =
    kind === 'triage'
      ? { opacity: 0, transform: 'translateX(56px)' }
      : kind === 'merge'
        ? { opacity: 0, transform: 'translateY(-10px) scale(.94)' }
        : { opacity: 0, transform: 'scale(.97)', filter: 'blur(4px)' };
  el.animate([{ opacity: 1, transform: 'none', filter: 'blur(0)' }, to], { duration: 380, easing: 'cubic-bezier(.4, 0, .2, 1)', fill: 'forwards' }).onfinish = () => el.remove();
}

/* ------------------------------ 稍后再做（default/main#303） ------------------------------ */

type SwipeState = 'drag' | 'armed' | 'out';

/** 行的横滑：只认 dir 那个方向（1 右、-1 左），横着拖（触屏、鼠标）过了阈值、或者甩得够快就松手即走，不露按钮；
    没到就弹回。竖着拖照常滚动；刚滑完的那次点击不算点开。算数和位移见 lib/swipe */
function useSwipe(dir: SwipeDir, enabled: boolean, onCommit: () => void) {
  const [dx, setDx] = useState<number | null>(null);
  const [state, setState] = useState<SwipeState | null>(null);
  const st = useRef<{ x: number; y: number; on: boolean; w: number; lx: number; lt: number; v: number; dx: number; el: HTMLElement } | null>(null);
  const ended = useRef(0);
  /** 原地收起高度的动画：没走成时撤掉 */
  const shrink = useRef<Animation | null>(null);
  const commit = useRef(onCommit);
  commit.current = onCommit;
  /** 拖动中挂在窗口上的兜底监听（松手没传到这一行时也能收尾）；结束时摘掉 */
  const off = useRef<(() => void) | null>(null);
  const unhook = () => {
    off.current?.();
    off.current = null;
  };
  const reset = () => {
    unhook();
    st.current = null;
    shrink.current?.cancel();
    shrink.current = null;
    setDx(null);
    setState(null);
  };
  /** 走：行滑出去（170ms），再在列表里原地把高度收到 0，收完才真的移走（commit）。
      一直在列表里排版：列表的底边、阴影、下面的行都跟着动，不会有一块脱离列表的色条 */
  const go = (el: HTMLElement, w: number) => {
    setState('out');
    setDx(dir * w);
    const row = el.closest<HTMLElement>('.ib-row');
    let sent = false;
    const done = () => {
      if (sent) return;
      sent = true;
      commit.current();
      // 移走之后这一行还在（例如服务端拒绝）：回到原位
      setTimeout(() => {
        if (row) row.style.overflow = '';
        reset();
      }, 700);
    };
    setTimeout(() => {
      if (!row || reduceMotion()) return done();
      row.style.overflow = 'hidden';
      shrink.current = row.animate([{ height: `${row.offsetHeight}px` }, { height: '0px' }], { duration: 240, easing: 'cubic-bezier(.4, 0, .2, 1)', fill: 'forwards' });
      shrink.current.onfinish = done;
      // 动画迟迟没播完（页面在后台之类）：照样移走
      setTimeout(done, 640);
    }, reduceMotion() ? 0 : 170);
  };
  /** 收尾：松手、丢了指针捕获、窗口上收到松手、移动时发现按键已经松开，都走这里；只收一次 */
  const finish = () => {
    const s = st.current;
    // 已经收过尾（窗口上和行上的松手都会到这里）：什么也不做，别把正在滑出去的行拉回来
    if (!s) return;
    unhook();
    st.current = null;
    if (!s.on) return;
    ended.current = performance.now();
    if (!swipeCommits(s.dx, s.v, dir, s.w)) return reset();
    go(s.el, s.w);
  };
  const fin = useRef(finish);
  fin.current = finish;
  useEffect(() => unhook, []);
  const handlers = {
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => {
      // 正在拖的时候又按下一次（另一根手指、连点）：不理，免得把这次拖动的状态冲掉、行停在半路
      if (st.current?.on) return;
      if (!enabled || e.button !== 0 || (e.target as HTMLElement).closest('button, input, textarea, a, [role="checkbox"]')) return;
      st.current = { x: e.clientX, y: e.clientY, on: false, w: e.currentTarget.offsetWidth, lx: e.clientX, lt: e.timeStamp, v: 0, dx: 0, el: e.currentTarget };
    },
    onPointerMove: (e: React.PointerEvent<HTMLElement>) => {
      const s = st.current;
      if (!s) return;
      // 按键已经松开了（松手发生在窗口外、没传进来）：就地收尾
      if (s.on && e.pointerType === 'mouse' && e.buttons === 0) return fin.current();
      const mx = e.clientX - s.x;
      const my = e.clientY - s.y;
      if (!s.on) {
        // 竖着拖、往反方向拖：交还给页面（滚动、选字）
        if (Math.abs(my) > 10 && Math.abs(my) > Math.abs(mx)) st.current = null;
        else if (Math.abs(mx) > 10) {
          if (mx * dir < 0) st.current = null;
          else {
            s.on = true;
            try {
              e.currentTarget.setPointerCapture(e.pointerId);
            } catch {
              /* 指针已经不在了：靠窗口上的监听收尾 */
            }
            const up = () => fin.current();
            window.addEventListener('pointerup', up, true);
            window.addEventListener('pointercancel', up, true);
            window.addEventListener('blur', up);
            off.current = () => {
              window.removeEventListener('pointerup', up, true);
              window.removeEventListener('pointercancel', up, true);
              window.removeEventListener('blur', up);
            };
          }
        }
        if (!st.current?.on) return;
      }
      const dt = e.timeStamp - s.lt;
      if (dt > 0) s.v = (e.clientX - s.lx) / dt;
      s.lx = e.clientX;
      s.lt = e.timeStamp;
      s.dx = swipeOffset(mx, dir, s.w);
      setDx(s.dx);
      setState(swipeCommits(s.dx, 0, dir, s.w) ? 'armed' : 'drag');
    },
    onPointerUp: () => fin.current(),
    onPointerCancel: reset,
    onLostPointerCapture: () => {
      if (st.current?.on) fin.current();
    },
  };
  return {
    state,
    handlers,
    /** 行上的按钮（收起、放回）：和滑过去松手一样地走；el 是这一行里的任意元素 */
    leave: (el: HTMLElement) => {
      if (st.current?.on || state === 'out') return;
      const line = el.closest<HTMLElement>('.ib-line');
      go(el, line?.offsetWidth ?? 0);
    },
    style: dx != null ? { transform: `translateX(${dx}px)`, transition: state === 'out' ? undefined : 'none' } : undefined,
    /** 这次点击要不要吞掉：刚滑完；行停在半路（不该出现，兜底）时点一下先弹回 */
    swallow: () => {
      if (performance.now() - ended.current < 300) return true;
      if (dx != null && state !== 'out') {
        reset();
        return true;
      }
      return false;
    },
  };
}

/** 滑动时行底下露出来的提示：写着松手会去哪；过了阈值变实色。只是提示，不能点 */
function SwipeHint({ dir, label, icon }: { dir: SwipeDir; label: string; icon: 'timer' | 'inbox' }) {
  return (
    <span className={cx('ib-swipe-hint', dir > 0 ? 'to-later' : 'to-inbox')} aria-hidden="true">
      <Icon name={icon} />
      {label}
    </span>
  );
}

/** 页头的大标题就是切换：「未整理」和「稍后再做」并排，正在看的那个是大字，另一个灰色小一号 */
function TitleTabs({ later, count, laterN }: { later: boolean; count: number | null; laterN: number | null }) {
  const pick = (l: boolean) => l !== later && setInbox({ later: l, checked: [], open: null });
  return (
    <h1 className="ib-title" role="tablist" aria-label="未整理 / 稍后再做">
      <button type="button" role="tab" aria-selected={!later} className={cx(!later && 'on')} onClick={() => pick(false)}>
        未整理
        {count != null && <span className="ib-total mono">{count}</span>}
      </button>
      <button type="button" role="tab" aria-selected={later} className={cx('ib-title-later', later && 'on')} onClick={() => pick(true)}>
        稍后再做
        {!!laterN && <span className="ib-later-n mono">{laterN}</span>}
      </button>
    </h1>
  );
}

/** 「稍后再做」页：按收起时间分组（最近收起的在前），行和「未整理」同一个样子；
    左滑放回、点开看详情，可以勾选后批量放回、打标签、删除；侧栏的筛选也管这一页 */
function LaterPage({ cards, labels, project, path }: { cards: Card[] | null; labels: Label[]; project: string; path: string }) {
  const now = useNow(60000);
  const filter = useUI(s => s.filter);
  const q = filter.trim().toLowerCase();
  const [checked, setChecked] = useState<number[]>([]);
  const lastCheck = useRef<number | null>(null);
  const list = useMemo(() => cards?.filter(c => matches(c, q, labels)) ?? null, [cards, q, labels]);
  const groups = useMemo(() => (list ? laterGroups(list, now) : []), [list, now]);
  const flat = useMemo(() => groups.flatMap(g => g.rows), [groups]);
  const wrap = useRef<HTMLDivElement>(null);
  const present = useMemo(() => new Set((cards ?? []).map(c => renderKey(c.id))), [cards]);
  useRowMotion(wrap, present, path, 'lt');
  // 放回、删掉、别的窗口改了：不在这一页的从勾选里去掉
  useEffect(() => setChecked(cs => (cs.some(id => !cards?.some(c => c.id === id)) ? cs.filter(id => cards?.some(c => c.id === id)) : cs)), [cards]);
  const check = (id: number, shift: boolean) => {
    setChecked(cur => {
      const on = !cur.includes(id);
      if (shift && lastCheck.current != null && flat.some(c => c.id === lastCheck.current)) {
        const a = flat.findIndex(c => c.id === lastCheck.current);
        const b = flat.findIndex(c => c.id === id);
        const range = flat.slice(Math.min(a, b), Math.max(a, b) + 1).map(c => c.id);
        return on ? [...new Set([...cur, ...range])] : cur.filter(c => !range.includes(c));
      }
      return on ? [...cur, id] : cur.filter(c => c !== id);
    });
    lastCheck.current = id;
  };
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest?.('input, textarea, select, [contenteditable]') || overlayOpen() || useData.getState().detailId != null) return;
      if (e.key === 'Escape' && checked.length) {
        e.preventDefault();
        setChecked([]);
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && flat.length) {
        e.preventDefault();
        setChecked(flat.map(c => c.id));
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [flat, checked.length]);

  if (!cards) return <RowsSkeleton n={3} />;
  return (
    <div ref={wrap}>
      {groups.map(g => {
        const ids = g.rows.map(c => c.id);
        const allOn = ids.every(id => checked.includes(id));
        const some = !allOn && ids.some(id => checked.includes(id));
        return (
          <section key={g.key} className="ib-grp">
            <div className="ib-gh">
              <Check
                on={allOn}
                some={some}
                label={allOn ? `取消勾选「${g.label}」` : `勾选「${g.label}」这一组`}
                onToggle={() => setChecked(cur => (allOn ? cur.filter(c => !ids.includes(c)) : [...new Set([...cur, ...ids])]))}
              />
              <span>{g.label}</span>
              <span className="ib-gh-n mono">{g.rows.length}</span>
            </div>
            <div className="ib-rows">
              {g.rows.map(c => (
                <LaterRow key={renderKey(c.id)} card={c} labels={labels} project={project} now={now} checked={checked.includes(c.id)} onCheck={check} />
              ))}
            </div>
          </section>
        );
      })}
      {!cards.length && (
        <div className="ib-empty">
          <div className="ib-later-art" aria-hidden="true">
            <Icon name="timer" />
          </div>
          <b>没有收起来的条目</b>
          <span>在「未整理」里往右滑一条，暂时不想处理的就收到这里。</span>
        </div>
      )}
      {cards.length > 0 && list && !list.length && (
        <div className="ib-empty">
          <b>没有符合的条目</b>
          <span>换个词试试，或者清除筛选。</span>
          <button className="btn sm" type="button" onClick={() => setFilter('')}>
            清除筛选
          </button>
        </div>
      )}
      <LaterBulk checked={checked} cards={cards} project={project} onClear={() => setChecked([])} />
    </div>
  );
}

function LaterRow({ card: c, labels, project, now, checked, onCheck }: { card: Card; labels: Label[]; project: string; now: number; checked: boolean; onCheck: (id: number, shift: boolean) => void }) {
  const saving = useCardBusy(c.id);
  const pending = useData(s => s.pending[`${s.path}#${c.id}`]);
  // 往左滑：直接放回「未整理」
  const sw = useSwipe(-1, !saving, () => void doUnlater([c.id], true));
  const text = snippet(c.body);
  return (
    <div data-lt={renderKey(c.id)} className={cx('ib-row', checked && 'checked', saving && 'saving', sw.state && `swipe-${sw.state}`)}>
      <div className="ib-swipe">
        <SwipeHint dir={-1} label="放回未整理" icon="inbox" />
        <div
          className="ib-line ib-later-line"
          role="button"
          tabIndex={0}
          aria-label={`#${c.id} ${c.title}`}
          style={sw.style}
          {...sw.handlers}
          onClick={e => {
            if (sw.swallow()) return;
            if (e.shiftKey) return onCheck(c.id, true);
            openCard(c.id);
          }}
          onKeyDown={e => {
            if (e.target !== e.currentTarget) return;
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              openCard(c.id);
            }
            if (e.key === 'x') onCheck(c.id, e.shiftKey);
          }}
        >
          <Check on={checked} label={`勾选 #${c.id} ${c.title}`} onToggle={shift => onCheck(c.id, shift)} />
          <span className="ib-id">#{c.id}</span>
          <ActorAvatar actor={c.createdBy} className="ib-av" />
          <span className="ib-t">
            <b>
              {saving && <i className="ib-saving" aria-label="保存中" />}
              <Title text={c.title} />
            </b>
            {text && <span>{text}</span>}
          </span>
          <span className="ib-labels">
            <LabelChips ids={c.labels} labels={labels} max={2} pending={pending} />
          </span>
          <span className="ib-acts">
            <Tip text="标签">
              <button
                type="button"
                className="icon-btn sm"
                aria-label={`给 #${c.id} 打标签`}
                disabled={saving}
                data-label-picker
                onClick={e => {
                  e.stopPropagation();
                  openPicker({
                    rect: e.currentTarget.getBoundingClientRect(),
                    project,
                    selected: c.labels,
                    onToggle: async (id, on) => {
                      try {
                        await (on ? addLabel(c.id, id) : removeLabel(c.id, id));
                      } catch (err) {
                        toastError(err);
                      }
                    },
                  });
                }}
              >
                <Icon name="tag" />
              </button>
            </Tip>
            <Tip text="删除（进回收站）">
              <button
                type="button"
                className="icon-btn sm ib-del"
                aria-label={`删除 #${c.id}`}
                disabled={saving}
                onClick={e => {
                  e.stopPropagation();
                  void doDelete([c.id], [c]);
                }}
              >
                <Icon name="trash" />
              </button>
            </Tip>
            <Tip text="放回未整理（左滑也行）">
              <button
                type="button"
                className="icon-btn sm"
                aria-label={`把 #${c.id} 放回未整理`}
                disabled={saving}
                onClick={e => {
                  e.stopPropagation();
                  // 和左滑一样：行往左滑出去、原地收起，再移走
                  sw.leave(e.currentTarget);
                }}
              >
                <Icon name="inbox" />
              </button>
            </Tip>
          </span>
          <span className="ib-meta">
            <span className="ib-age" title="收起的时间">{c.laterAt ? ago(c.laterAt, now) : ''}</span>
          </span>
        </div>
      </div>
    </div>
  );
}

/** 「稍后再做」勾选后的批量条：标签、删除、放回「未整理」 */
function LaterBulk({ checked, cards, project, onClear }: { checked: number[]; cards: Card[]; project: string; onClear: () => void }) {
  const n = checked.length;
  const bulkLabels = (rect: DOMRect) => {
    const ids = checked;
    const all = useData.getState().board?.cards ?? {};
    const common = [...new Set(ids.flatMap(id => all[id]?.labels ?? []))].filter(l => ids.every(id => all[id]?.labels.includes(l)));
    openPicker({
      rect,
      project,
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
          onClear();
          void doUnlater(ids);
        }}
      >
        放回「未整理」
      </button>
      <Tip text="取消选择 · esc">
        <button type="button" className="icon-btn sm" aria-label="取消选择" data-tip-pos="top" onClick={onClear}>
          <Icon name="close" />
        </button>
      </Tip>
    </div>
  );
}
