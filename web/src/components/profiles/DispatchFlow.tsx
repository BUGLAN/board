/* ==========================================================================
   派发对话框（按角色，PRD §9.1「派发」）：和分工页同一种画法——卡片（命中的规则）→ 角色 → 机器三栏连线。
   一张：卡片详情、机器页、「接下来」的派发 / 改派 / 拆解；多张：白板批量条的「派送」，卡片栏按命中的规则分组，
   每组虚线连到它按规则会交给的角色。按规则会交给的角色预先选中（多张时取按规则交给它的卡最多的那个）；
   点一个角色换，悬停先看它会去哪。多张也只选一个角色，能派的卡都派给它。
   机器栏最上面是「随机」（默认）：在能跑这个角色的机器里挑空位最多的，都满就排队；点一台机器指定它，点「随机」换回去。
   手动派发不看接单时段。复制提示词在卡片上就有，这里不放。弹出框高度不随选择变
   派发要服务端排进队列才有记录：按钮转圈、禁用直到请求返回、随后的刷新也完成，机器页的旧轮询不会把派发画回去。
   多张时按单卡同一套规则跳过不能派的（lib/boardSelect dispatchPlan），其余一次请求派整批（服务端按会话分组），单张失败不影响别的
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { bulkDispatchResult, bulkDispatchToast, dispatchPlan } from '../../lib/boardSelect';
import { blockOf, dirFresh, hostsOf } from '../../lib/hosts';
import { groupPick, hostsFor, OP_WORDS, pickHost, profilesOf, ruleGroups, short, soonest, type HostWhy, type RuleGroup } from '../../lib/profiles';
import { serverMinute } from '../../lib/sched';
import type { Dispatch, HostInfo } from '../../lib/types';
import { cx, isImeKey } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { openDispatch, toast, toastError } from '../../store/ui';
import { useNow } from '../bits';
import { OsGlyph } from '../hosts/bits';
import { Icon } from '../Icon';
import { PMark, Slots, colorOf, useProviders } from './bits';

type ZEdge = { from: string; to: string; dash: boolean; lit: boolean; color: string };

/** 一次派发的去向写成一句：「win · 批量」「任意机器 · 批量」 */
const hostWords = (d: Dispatch) => (d.host ? d.host : '任意机器');

/** 这块白板所在项目的目录在这台机器上克隆失败时，返回要重新克隆的白板和地址（和机器详情里的「重新克隆」是同一件事）。
    只看还没过期、状态是 missing、克隆失败的目录；同一个项目的几块白板共用一个目录，所以按项目匹配 */
function failedClone(h: HostInfo, board: string): { board: string; url: string } | null {
  const proj = board.split('/')[0];
  for (const d of h.dirs) {
    if (!dirFresh(h, d)) continue;
    if (d.status === 'missing' && d.clone?.state === 'failed' && d.board.split('/')[0] === proj) return { board: d.board, url: d.clone.url };
  }
  return null;
}

export function DispatchFlow({ cards, move, on }: { cards: number[]; move?: number; on: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const board = useData(s => s.board);
  const now = useNow(30_000);
  useProviders();
  const minute = serverMinute(ws, now);
  const profiles = profilesOf(ws);
  const hosts = hostsOf(ws);
  const many = cards.length > 1;
  const card = cards[0];
  const entry = board?.cards[card];
  const rules = ws?.sched?.rules ?? [];
  // 多张：能派的和跳过的（锁定、被持有、已有排队或在跑的……），按单卡同一套规则
  const plan = useMemo(() => {
    if (!many) return null;
    const active = new Set((ws?.dispatches ?? []).filter(d => d.board === path && d.state !== 'done').map(d => d.card));
    return dispatchPlan(board?.statuses ?? [], board?.cards ?? {}, cards, active);
  }, [many, ws?.dispatches, path, board, cards]);
  // 「卡片」栏：一张时就是它命中的规则；多张时是能派的卡按规则分组（跳过的不算）
  const groups = useMemo(
    () => ruleGroups(ws, path, (plan ? plan.ids : [card]).map(id => ({ id, labels: board?.cards[id]?.labels ?? [] }))),
    [ws, path, board, plan, card],
  );
  const ruleWords = (i: number | null) => {
    if (i == null) return '没有命中的规则：自己挑一个角色';
    const r = rules[i];
    return `规则 ${i + 1}${r ? `：${OP_WORDS[r.op]}${r.labels.length ? ` ${r.labels.join('、')}` : ''}` : ''}`;
  };
  // 按规则交给这个角色的卡有几张
  const byRule = (id: string) => groups.reduce((n, g) => n + (g.profile === id ? g.ids.length : 0), 0);
  const moving = move != null ? (ws?.dispatches ?? []).find(d => d.id === move) : undefined;
  const split = !many && entry?.column === 'inbox';

  // 这块白板接不了的机器（没有工作空间、目录不对）也跑不了
  const list = (p: string): HostWhy[] =>
    hostsFor(ws, p, minute, false).map(x => {
      const b = blockOf(ws, x.host.name, path);
      return b && x.can ? { ...x, can: false, ok: false, why: b.why, tone: 'bad' } : x;
    });
  const [prof, setProf] = useState<string>(
    () =>
      moving?.profile ||
      groupPick(groups) ||
      // 多张都没命中规则：先选第一个有机器能跑的
      (many ? profiles.find(x => list(x.id).some(h => h.can))?.id : undefined) ||
      profiles[0]?.id ||
      '',
  );
  // 指定的机器（id）；0 = 随机
  const [host, setHost] = useState<number>(moving && !moving.any ? moving.hostId : 0);
  const [hover, setHover] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  // 正在请求重新克隆的机器 id（这一行按钮转圈、禁用）
  const [rcBusy, setRcBusy] = useState<number | null>(null);
  const map = useRef<HTMLDivElement>(null);
  const [paths, setPaths] = useState<(ZEdge & { d: string; k: string })[]>([]);
  const close = () => openDispatch(null);

  const focus = hover ?? prof;
  const fp = profiles.find(x => x.id === focus);
  const fhw = fp ? list(fp.id) : [];
  const auto = pickHost(ws, fhw);
  // 在看的就是选中的角色，并且指定了机器；否则是「随机」
  const pinnedHere = focus === prof && host !== 0;
  const can = fhw.some(x => x.can);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || isImeKey(e)) return;
      e.stopPropagation();
      e.preventDefault();
      close();
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, []);

  const edges = useMemo(() => {
    const e: ZEdge[] = [];
    groups.forEach((g, i) => {
      const sp = g.profile ? profiles.find(x => x.id === g.profile) : undefined;
      if (sp && sp.id !== focus) e.push({ from: `g${i}`, to: `p${sp.id}`, dash: true, lit: false, color: colorOf(sp) });
      if (fp) e.push({ from: `g${i}`, to: `p${fp.id}`, dash: false, lit: true, color: colorOf(fp) });
    });
    if (fp && can) e.push({ from: `p${fp.id}`, to: 'hany', dash: false, lit: !pinnedHere, color: colorOf(fp) });
    for (const x of fhw) if (x.can) e.push({ from: `p${focus}`, to: `h${x.host.id}`, dash: !x.ok, lit: pinnedHere && x.host.id === host, color: colorOf(fp) });
    return e;
    // fhw 由 ws、focus 算出
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ws, focus, groups, pinnedHere, host, minute]);

  useLayoutEffect(() => {
    const el = map.current;
    if (!el) return;
    const draw = () => {
      if (el.offsetWidth < 480) return setPaths([]);
      const box = el.getBoundingClientRect();
      // 对话框打开时有缩放动画：按缩放比例换回布局坐标
      const k = box.width / el.offsetWidth || 1;
      const at = (key: string) => el.querySelector<HTMLElement>(`[data-k="${key}"]`)?.getBoundingClientRect();
      setPaths(
        edges.flatMap((e, i) => {
          const a = at(e.from);
          const b = at(e.to);
          if (!a || !b) return [];
          const x1 = (a.right - box.left) / k;
          const y1 = (a.top + a.height / 2 - box.top) / k;
          const x2 = (b.left - box.left) / k;
          const y2 = (b.top + b.height / 2 - box.top) / k;
          const dx = Math.max(20, (x2 - x1) * 0.5);
          return [{ ...e, k: `${e.from}>${e.to}#${i}`, d: `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}` }];
        }),
      );
    };
    draw();
    // 测试环境（jsdom）没有 ResizeObserver：只画一次
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(draw);
    ro.observe(el);
    return () => ro.disconnect();
  }, [edges]);

  const p = profiles.find(x => x.id === prof);
  const hw = p ? list(p.id) : [];
  const pinned = host ? hw.find(x => x.host.id === host) : undefined;
  const got = pinned ? (pinned.ok ? pinned : null) : pickHost(ws, hw);
  // 多张时能派几张
  const n = plan ? plan.ids.length : 1;
  const where = p ? `派给「${p.name}」· ${pinned ? pinned.host.name : '随机'}` : '';
  const say = !p
    ? '先选一个角色'
    : !n
      ? '没有可派发的卡片'
      : !hw.some(x => x.can)
        ? '没有机器能跑这个角色'
        : many
          ? `${n} 张${where}：按会话分组排队，${got ? `${got.host.name} ${got.why}，有空位的马上开始。` : pinned ? `${pinned.host.name} ${pinned.why}，排队等它。` : `都要排队，${soonest(hw)}。`}`
          : `${where}：${got ? `${got.host.name} ${got.why}，马上开始。` : pinned ? `${pinned.host.name} ${pinned.why}，排队等它。` : `排队，${soonest(hw)}。`}`;
  const verb = move != null ? '改派' : split ? '拆解' : '派发';
  const heading = many ? `派发 ${cards.length} 张卡片` : `${verb} #${card}`;

  // 这台机器上这块白板的目录克隆失败了：就地点「重新克隆」，不用再翻到机器详情。请求发出后服务端会把它记成
  // 「正在克隆」，这一行的原因跟着变、按钮消失；克隆好以后这台机器就能选了
  const reclone = async (h: HostInfo, rc: { board: string; url: string }) => {
    setRcBusy(h.id);
    try {
      await optimistic(null, () => api.cloneHostDir(h.name, rc.board, rc.url));
      toast({ type: 'success', title: `让 ${h.name} 重新克隆 ${rc.board}`, text: '开始后这一行写「正在克隆」，克隆好就能派' });
    } catch (e) {
      toastError(e);
    } finally {
      setRcBusy(null);
    }
  };

  const submit = async () => {
    if (!p || !n) return;
    setSending(true);
    try {
      const to = pinned ? pinned.host.name : '';
      if (plan) {
        // 多张：一个请求派整批，都带这个角色，服务端按机器「一个会话最多做几张」排成几个会话；单张失败不影响别的。返回后才关对话框
        const { ids, skipped } = plan;
        const result = bulkDispatchResult(await optimistic(null, () => api.dispatchBatch(path, ids, to, undefined, p.id)));
        close();
        toast(bulkDispatchToast(result, skipped, where));
        return;
      }
      const d = await optimistic(null, () =>
        move != null ? api.moveDispatch(move, to, undefined, p.id) : api.dispatch(path, card, to, undefined, p.id),
      );
      close();
      toast({
        type: 'success',
        title: move != null ? `#${card} 已改派到 ${hostWords(d)} · ${p.name}` : split ? `#${card} 已派给 ${hostWords(d)} · ${p.name} 拆解` : `#${card} 已派发到 ${hostWords(d)} · ${p.name}`,
        text: got ? '机器有空位，马上开始' : '排队：等一台能跑这个角色的机器空出来',
      });
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };

  const groupNode = (g: RuleGroup, i: number) => (
    <div
      key={g.rule ?? 'none'}
      data-k={`g${i}`}
      className="pfz-card"
      title={many ? `${ruleWords(g.rule)}\n${g.ids.map(id => `#${id} ${board?.cards[id]?.title ?? ''}`).join('\n')}` : ruleWords(g.rule)}
    >
      <Icon name="filter" />
      <b>{g.rule != null ? `规则 ${g.rule + 1}` : '没命中规则'}</b>
      {many && <small>{g.ids.length} 张</small>}
    </div>
  );

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog dp-dialog pfd flow" role="dialog" aria-modal="true" aria-label={heading}>
        <h3>
          {heading}
          <small className="pfz-title">{many ? '逐张排队派发' : entry?.title}</small>
        </h3>
        {plan && (
          <p className={cx('dp-note', 'dp-plan', !plan.ids.length && 'bad')} role="status">
            {plan.ids.length ? `可派发 ${plan.ids.length} 张` : '没有可派发的卡片'}
            {plan.skipped.length ? `；跳过 ${plan.skipped.length} 张：${plan.skipped.map(x => `#${x.id} ${x.why}`).join('；')}` : ''}
          </p>
        )}
        {hosts.length === 0 ? (
          <p className="dp-note">还没有机器。在要干活的机器上运行 board host，它会出现在这里。</p>
        ) : (
          <div className="pfa-map pfz-map" ref={map} onMouseLeave={() => setHover(null)}>
            <svg className="pfa-lines" aria-hidden="true">
              {paths.map(x => (
                <path key={x.k} d={x.d} className={cx(x.dash && 'dash', x.lit ? 'lit' : 'dim')} style={{ ['--c' as string]: x.color }} />
              ))}
            </svg>
            <div className="pfa-col pfz-col">
              {groups.length ? (
                groups.map(groupNode)
              ) : (
                <div className="pfz-card">
                  <Icon name="filter" />
                  <b>没有可派发的卡片</b>
                </div>
              )}
            </div>
            <div className="pfa-col pfz-col">
              {profiles.map(x => {
                const able = list(x.id).some(h => h.can);
                const ruled = byRule(x.id);
                return (
                  <button
                    key={x.id}
                    type="button"
                    data-k={`p${x.id}`}
                    className={cx('pfz-prof', prof === x.id && 'on', focus === x.id && 'lit')}
                    style={{ ['--c' as string]: colorOf(x) }}
                    disabled={!able}
                    title={able ? undefined : '没有机器能跑这个角色'}
                    onClick={() => (setProf(x.id), setHost(0))}
                    onMouseEnter={() => setHover(x.id)}
                  >
                    <PMark p={x} />
                    <b>{x.name}</b>
                    {ruled > 0 && <span className="pfx-tag">{many ? `按规则 ${ruled} 张` : '按规则'}</span>}
                    <small>{short(x)}</small>
                  </button>
                );
              })}
            </div>
            <div className="pfa-col pfz-col">
              <button
                type="button"
                data-k="hany"
                className={cx('pfz-host pfz-any', !pinnedHere && 'on')}
                style={{ ['--c' as string]: colorOf(fp) }}
                disabled={focus !== prof || !can}
                title="在能跑这个角色的机器里挑空位最多的；都满了就排队，谁先空出来给谁"
                onClick={() => setHost(0)}
              >
                <Dice />
                <b>随机</b>
                <small>{!can ? '没有机器能跑' : auto ? `现在会去 ${auto.host.name}` : '都满，排队'}</small>
              </button>
              {fhw.map(x => {
                // 只有「接不了这块白板」的原因（不是这个角色跑不了）才可能是克隆失败：对一下显示的原因就是服务端给的 block
                const blk = blockOf(ws, x.host.name, path);
                const rc = x.why === blk?.why ? failedClone(x.host, path) : null;
                const inner = (
                  <>
                    <OsGlyph os={x.host.os} small />
                    <b>{x.host.name}</b>
                    <Slots h={x.host} ws={ws} />
                    <small>{x.why}</small>
                  </>
                );
                // 克隆失败：这一行本来就不能选，就地给一个「重新克隆」，免得人只看到原因、没有地方重试
                if (rc) {
                  return (
                    <div key={x.host.id} data-k={`h${x.host.id}`} className={cx('pfz-host', 'blocked', `t-${x.tone}`)} style={{ ['--c' as string]: colorOf(fp) }} title={`${x.host.name}：${x.why}`}>
                      {inner}
                      <button
                        type="button"
                        className={cx('pfz-reclone', rcBusy === x.host.id && 'is-busy')}
                        disabled={rcBusy === x.host.id || !x.host.online}
                        title={x.host.online ? `重新克隆 ${rc.board} 到这台机器` : '机器离线：连上后才能在界面上重新克隆'}
                        onClick={() => void reclone(x.host, rc)}
                      >
                        {rcBusy === x.host.id ? '正在请求…' : '重新克隆'}
                      </button>
                    </div>
                  );
                }
                return (
                  <button
                    key={x.host.id}
                    type="button"
                    data-k={`h${x.host.id}`}
                    className={cx('pfz-host', !x.host.online && 'off', focus === prof && host === x.host.id && 'on', !x.can && 'cant', `t-${x.tone}`)}
                    style={{ ['--c' as string]: colorOf(fp) }}
                    disabled={!x.can || focus !== prof}
                    title={`${x.host.name}：${x.why}`}
                    onClick={() => setHost(x.host.id)}
                  >
                    {inner}
                  </button>
                );
              })}
            </div>
          </div>
        )}
        <div className="pfz-foot">
          <p className={cx('dp-note pfd-say', got && n ? 't-ok' : 't-wait')}>{say}</p>
          <button className="btn sm" type="button" onClick={close}>
            取消
          </button>
          <button className={cx('btn sm brand', sending && 'is-busy')} type="button" disabled={sending || !p || !n || !hw.some(x => x.can)} aria-busy={sending || undefined} onClick={submit}>
            <Icon name="dispatch" />
            {many ? `派送 ${n} 张` : verb}
          </button>
        </div>
      </div>
    </div>
  );
}

/** 「随机」的图标：一颗骰子，和系统图标一样大 */
function Dice() {
  return (
    <svg className="pfz-dice" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="2" y="2" width="12" height="12" rx="3" fill="none" stroke="currentColor" strokeWidth="1.4" />
      <circle cx="5.6" cy="5.6" r="1.1" fill="currentColor" />
      <circle cx="10.4" cy="10.4" r="1.1" fill="currentColor" />
      <circle cx="8" cy="8" r="1.1" fill="currentColor" />
    </svg>
  );
}
