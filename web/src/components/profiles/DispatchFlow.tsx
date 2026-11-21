/* ==========================================================================
   派发对话框（按角色，PRD §9.1「派发」）：和分工页同一种画法，只画这一张卡——
   这张卡（标签、命中的规则）→ 角色 → 机器三栏连线。按规则会交给的角色预先选中；点一个角色换，悬停先看它会去哪。
   机器栏最上面是「随机」（默认）：在能跑这个角色的机器里挑空位最多的，都满就排队；点一台机器指定它，点「随机」换回去。
   手动派发不看接单时段。复制提示词在卡片上就有，这里不放。弹出框高度不随选择变
   派发要服务端排进队列才有记录：按钮转圈、禁用直到返回；返回后当即在卡片和机器页上画出这条派发
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { blockOf, hostsOf } from '../../lib/hosts';
import { hostsFor, labelNames, pickHost, profilesOf, routeOf, short, soonest, type HostWhy } from '../../lib/profiles';
import { serverMinute } from '../../lib/sched';
import type { Dispatch } from '../../lib/types';
import { cx } from '../../lib/util';
import { hold, refresh, useData } from '../../store/data';
import { openDispatch, toast, toastError } from '../../store/ui';
import { useNow } from '../bits';
import { OsGlyph } from '../hosts/bits';
import { Icon } from '../Icon';
import { NameChips, PMark, Slots, colorOf, useProviders } from './bits';

type ZEdge = { from: string; to: string; dash: boolean; lit: boolean; color: string };

/** 一次派发的去向写成一句：「win · 批量」「任意机器 · 批量」 */
const hostWords = (d: Dispatch) => (d.host ? d.host : '任意机器');

export function DispatchFlow({ card, move, on }: { card: number; move?: number; on: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const entry = useData(s => s.board?.cards[card]);
  const now = useNow(30_000);
  useProviders();
  const minute = serverMinute(ws, now);
  const profiles = profilesOf(ws);
  const hosts = hostsOf(ws);
  const names = useMemo(() => labelNames(ws, path, entry?.labels ?? []), [ws, path, entry?.labels]);
  const rules = ws?.sched?.rules ?? [];
  const route = routeOf(names, rules);
  const moving = move != null ? (ws?.dispatches ?? []).find(d => d.id === move) : undefined;
  const suggested = route ? profiles.find(p => p.id === route.profile) : undefined;
  const split = entry?.column === 'inbox';
  const [prof, setProf] = useState<string>(moving?.profile || suggested?.id || profiles[0]?.id || '');
  // 指定的机器（id）；0 = 随机
  const [host, setHost] = useState<number>(moving && !moving.any ? moving.hostId : 0);
  const [hover, setHover] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const map = useRef<HTMLDivElement>(null);
  const [paths, setPaths] = useState<(ZEdge & { d: string; k: string })[]>([]);
  const close = () => openDispatch(null);

  // 这块白板接不了的机器（没有工作空间、目录不对）也跑不了
  const list = (p: string): HostWhy[] =>
    hostsFor(ws, p, minute, false).map(x => {
      const b = blockOf(ws, x.host.name, path);
      return b && x.can ? { ...x, can: false, ok: false, why: b.why, tone: 'bad' } : x;
    });
  const focus = hover ?? prof;
  const fp = profiles.find(x => x.id === focus);
  const fhw = fp ? list(fp.id) : [];
  const auto = pickHost(ws, fhw);
  // 在看的就是选中的角色，并且指定了机器；否则是「随机」
  const pinnedHere = focus === prof && host !== 0;
  const can = fhw.some(x => x.can);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      e.preventDefault();
      close();
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, []);

  const edges = useMemo(() => {
    const e: ZEdge[] = [];
    if (suggested && suggested.id !== focus) e.push({ from: 'card', to: `p${suggested.id}`, dash: true, lit: false, color: colorOf(suggested) });
    if (fp) e.push({ from: 'card', to: `p${fp.id}`, dash: false, lit: true, color: colorOf(fp) });
    if (fp && can) e.push({ from: `p${fp.id}`, to: 'hany', dash: false, lit: !pinnedHere, color: colorOf(fp) });
    for (const x of fhw) if (x.can) e.push({ from: `p${focus}`, to: `h${x.host.id}`, dash: !x.ok, lit: pinnedHere && x.host.id === host, color: colorOf(fp) });
    return e;
    // fhw 由 ws、focus 算出
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ws, focus, suggested?.id, pinnedHere, host, minute]);

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
  const say = !p
    ? '先选一个角色'
    : !hw.some(x => x.can)
      ? '没有机器能跑这个角色'
      : `派给「${p.name}」· ${pinned ? pinned.host.name : '随机'}：${got ? `${got.host.name} ${got.why}，马上开始。` : pinned ? `${pinned.host.name} ${pinned.why}，排队等它。` : `排队，${soonest(hw)}。`}`;

  const submit = async () => {
    if (!p) return;
    setSending(true);
    try {
      const to = pinned ? pinned.host.name : '';
      const d = move != null ? await api.moveDispatch(move, to, undefined, p.id) : await api.dispatch(path, card, to, undefined, p.id);
      close();
      toast({
        type: 'success',
        title: move != null ? `#${card} 已改派到 ${hostWords(d)} · ${p.name}` : split ? `#${card} 已派给 ${hostWords(d)} · ${p.name} 拆解` : `#${card} 已派发到 ${hostWords(d)} · ${p.name}`,
        text: got ? '机器有空位，马上开始' : '排队：等一台能跑这个角色的机器空出来',
      });
      void hold({ ws: ws => ({ ...ws, dispatches: [...(ws.dispatches ?? []).filter(x => x.id !== d.id), d] }) }, refresh());
    } catch (e) {
      toastError(e);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className="dialog dp-dialog pfd flow" role="dialog" aria-modal="true" aria-label={`${move != null ? '改派' : split ? '拆解' : '派发'} #${card}`}>
        <h3>
          {move != null ? '改派' : split ? '拆解' : '派发'} #{card}
          <small className="pfz-title">{entry?.title}</small>
        </h3>
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
              <div data-k="card" className="pfz-card" title={route ? `规则 ${route.rule + 1}` : '没有命中的规则：自己挑一个角色'}>
                <span className="pfz-labels">{names.length ? <NameChips names={names} /> : <small>没有标签</small>}</span>
                <small className="pfz-rule">{route ? `规则 ${route.rule + 1}` : '没命中规则'}</small>
              </div>
            </div>
            <div className="pfa-col pfz-col">
              {profiles.map(x => {
                const able = list(x.id).some(h => h.can);
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
                    {suggested?.id === x.id && <span className="pfx-tag">按规则</span>}
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
              {fhw.map(x => (
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
                  <OsGlyph os={x.host.os} small />
                  <b>{x.host.name}</b>
                  <Slots h={x.host} ws={ws} />
                  <small>{x.why}</small>
                </button>
              ))}
            </div>
          </div>
        )}
        <div className="pfz-foot">
          <p className={cx('dp-note pfd-say', got ? 't-ok' : 't-wait')}>{say}</p>
          <button className="btn sm" type="button" onClick={close}>
            取消
          </button>
          <button className={cx('btn sm brand', sending && 'is-busy')} type="button" disabled={sending || !p || !hw.some(x => x.can)} aria-busy={sending || undefined} onClick={submit}>
            <Icon name="dispatch" />
            {move != null ? '改派' : split ? '拆解' : '派发'}
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
