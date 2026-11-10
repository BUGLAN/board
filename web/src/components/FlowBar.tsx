/* ==========================================================================
   工作流条（PRD §10.3）：把工作流画成有向图
   - 主线排第一行，其余状态挂在第二行；连线是直角折线（排版见 lib/flowGraph）
   - 连线全靠拖：从状态右边缘的圆点拖到另一个状态上即连线；拖连线的箭头改连，拖到空白处删除；
     点连线选中后按 Delete 或点 × 也能删除
   - 拖动状态调整顺序；点状态打开设置（名称、颜色、规则、删除）
   - 连线只约束 agent：agent 只能沿连线移动卡片，人不受限制
   - readOnly（设置页「白板规则」）：同样的图，只看不改：没有连线把手、状态菜单、拖动、添加状态、白板设置
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../lib/api';
import { CH, edgeId, layoutFlow, midOf, pathD, type Pt } from '../lib/flowGraph';
import { statusColor, STATUS_COLORS, viewOf, VIEWS, type FlowQ } from '../lib/schema';
import type { Status } from '../lib/types';
import { cx, replay } from '../lib/util';
import { flowKey, optimistic, useBusy, useData } from '../store/data';
import { toastError } from '../store/ui';
import { BoardSettings } from './BoardSettings';
import { Icon } from './Icon';

/** 改工作流：当帧生效（给白板的状态列表叠一层补丁），被拒绝时改回来并提示 */
const run = (statuses: (list: Status[]) => Status[], req: () => Promise<unknown>) =>
  optimistic({ board: b => ({ ...b, statuses: statuses(b.statuses) }), keys: [flowKey(useData.getState().path)] }, req).catch(toastError);

/** 加 / 去连线后的状态列表；连线按工作流顺序排（与服务端一致） */
const withEdges = (add: [string, string][], remove: [string, string][]) => (list: Status[]) => {
  const rank = new Map(list.map((s, i) => [s.key, i]));
  return list.map(s => {
    const out = s.to.filter(t => !remove.some(([a, b]) => a === s.key && b === t));
    for (const [a, b] of add) if (a === s.key && !out.includes(b)) out.push(b);
    out.sort((x, y) => (rank.get(x) ?? 0) - (rank.get(y) ?? 0));
    return out.length === s.to.length && out.every((t, i) => t === s.to[i]) ? s : { ...s, to: out };
  });
};

/** 拖出连线：from 是起点；改连时 replace 是原来那条连线 */
interface Conn {
  from: string;
  replace: string | null;
  start: Pt;
  end: Pt;
  target: string | null;
}
/** 拖动状态调整顺序 */
interface Lift {
  key: string;
  dx: number;
  dy: number;
}

/** counts：每个状态的卡片数（快照还没到时用工作区里的计数）；readOnly：只读展示 */
export function FlowBar({ counts, path, f, readOnly = false }: { counts: Record<string, number>; path: string; f: FlowQ; readOnly?: boolean }) {
  const [menu, setMenu] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [hover, setHover] = useState<string | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [conn, setConn] = useState<Conn | null>(null);
  const [lift, setLift] = useState<Lift | null>(null);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [addW, setAddW] = useState(96);
  const canvas = useRef<HTMLDivElement>(null);
  // 工作流的修改还没保存完：先不能再改（连线、顺序、设置），保存完再改
  const busy = useBusy(flowKey(path));
  const chips = useRef<Record<string, HTMLElement | null>>({});
  const addEl = useRef<HTMLElement | null>(null);
  const moved = useRef(false);
  const [settings, setSettings] = useState(false);
  const settingsEl = useRef<HTMLButtonElement>(null);

  // 只读时没有「+ 添加状态」，不给它留位置
  const g = useMemo(() => layoutFlow(f.list, widths, readOnly ? 0 : addW), [f.list, widths, addW, readOnly]);

  // 量出每个状态胶囊的宽度，再排版。白板页被隐藏（切到别的 Tab）时量出来都是 0：不记，切回来不会先挤成一团再展开
  useLayoutEffect(() => {
    if (!canvas.current?.offsetParent) return;
    const next: Record<string, number> = {};
    let changed = false;
    for (const s of f.list) {
      const w = chips.current[s.key]?.offsetWidth ?? 0;
      next[s.key] = w;
      if (widths[s.key] !== w) changed = true;
    }
    if (changed || Object.keys(widths).length !== f.list.length) setWidths(next);
    const aw = addEl.current?.offsetWidth ?? 96;
    if (aw !== addW) setAddW(aw);
  });

  // 选中的连线：Delete 删除，Esc 取消
  useEffect(() => {
    if (!sel) return;
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest('input, textarea, [contenteditable="true"]')) return;
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        removeEdge(sel);
      } else if (e.key === 'Escape') {
        e.stopPropagation();
        setSel(null);
      }
    };
    const down = (e: PointerEvent) => {
      if (!(e.target as Element).closest('.wf-hit, .wf-edge-del')) setSel(null);
    };
    addEventListener('keydown', key, true);
    addEventListener('pointerdown', down);
    return () => {
      removeEventListener('keydown', key, true);
      removeEventListener('pointerdown', down);
    };
  }, [sel]);

  const split = (id: string) => id.split('>') as [string, string];
  const removeEdge = (id: string) => {
    setSel(null);
    run(withEdges([], [split(id)]), () => api.updateEdges(path, [], [split(id)]));
  };

  /** 画布坐标 */
  const local = (e: { clientX: number; clientY: number }): Pt => {
    const r = canvas.current!.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  };
  const hit = ([x, y]: Pt) =>
    f.list.find(s => {
      const b = g.boxes[s.key];
      return b && x >= b.x - 4 && x <= b.x + b.w + 4 && y >= b.y - 4 && y <= b.y + CH + 4;
    })?.key ?? null;

  /** 从圆点拖出新连线，或拖已有连线的箭头改连 */
  const startConnect = (from: string, replace: string | null, start: Pt) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    setMenu(null);
    setSel(null);
    let cur: Conn = { from, replace, start, end: local(e), target: null };
    setConn(cur);
    const mv = (ev: PointerEvent) => {
      const p = local(ev);
      const k = hit(p);
      cur = { ...cur, end: p, target: k && k !== from ? k : null };
      setConn(cur);
    };
    const up = () => {
      removeEventListener('pointermove', mv);
      removeEventListener('pointerup', up);
      setConn(null);
      const t = cur.target;
      const add: [string, string][] = [];
      const remove: [string, string][] = [];
      if (replace) {
        if (t && edgeId(from, t) === replace) return; // 放回原处
        remove.push(split(replace));
      }
      if (t && !f.hasEdge(from, t)) add.push([from, t]);
      if (!add.length && !remove.length) return;
      if (t) setSel(edgeId(from, t));
      run(withEdges(add, remove), () => api.updateEdges(path, add, remove));
    };
    addEventListener('pointermove', mv);
    addEventListener('pointerup', up);
  };

  /** 拖动状态调整顺序：松手时按横向位置排进工作流顺序 */
  const startLift = (k: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const x0 = e.clientX,
      y0 = e.clientY;
    moved.current = false;
    let cur: Lift = { key: k, dx: 0, dy: 0 };
    const mv = (ev: PointerEvent) => {
      if (!moved.current) {
        if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 4) return;
        moved.current = true;
        setMenu(null);
      }
      cur = { key: k, dx: ev.clientX - x0, dy: ev.clientY - y0 };
      setLift(cur);
    };
    const up = () => {
      removeEventListener('pointermove', mv);
      removeEventListener('pointerup', up);
      setLift(null);
      if (!moved.current) return;
      const b = g.boxes[k];
      const center = b.x + b.w / 2 + cur.dx;
      const others = f.list.filter(s => s.key !== k);
      const i = others.filter(s => g.boxes[s.key].x + g.boxes[s.key].w / 2 < center).length;
      const order = [...others.slice(0, i).map(s => s.key), k, ...others.slice(i).map(s => s.key)];
      if (order.join() !== f.list.map(s => s.key).join())
        run(
          list => order.map(k => list.find(s => s.key === k)).filter((s): s is Status => !!s).concat(list.filter(s => !order.includes(s.key))),
          () => api.reorderStatuses(path, order),
        );
    };
    addEventListener('pointermove', mv);
    addEventListener('pointerup', up);
  };

  const related = (id: string) => {
    const k = hover ?? (conn && !conn.replace ? conn.from : null);
    if (!k) return null;
    const [a, b] = split(id);
    return a === k || b === k;
  };
  const selGeo = sel ? g.edges.find(e => e.id === sel) : undefined;

  return (
    <div className={cx('wf-bar wf-graph', readOnly && 'wf-ro', conn && 'connecting', busy && 'busy')} aria-busy={busy || undefined}>
      {!readOnly && (
        <span className="wf-label" style={{ marginTop: g.boxes[f.list[0]?.key]?.y ?? 0 }}>
          工作流
        </span>
      )}
      <div className="wf-scroll">
        <div className="wf-canvas" ref={canvas} style={{ width: g.width + 16, height: g.height }}>
          <svg className="wf-edges" width={g.width + 16} height={g.height} aria-hidden="true">
            <defs>
              {(['n', 'h', 's'] as const).map(m => (
                <marker key={m} id={`wf-ah-${m}`} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto">
                  <path d="M0,0.5 L7.5,4 L0,7.5 z" className={`wf-ah ${m}`} />
                </marker>
              ))}
            </defs>
            {g.add.from && !readOnly && (
              <path className="wf-edge add" d={pathD([g.add.from, [g.add.x - 4, g.add.from[1]]])} markerEnd="url(#wf-ah-n)" />
            )}
            {g.edges.map(e => {
              const r = related(e.id);
              const on = sel === e.id;
              const hidden = conn?.replace === e.id;
              return (
                <g key={e.id} style={hidden ? { opacity: 0.15 } : undefined}>
                  <path
                    className={cx('wf-edge', on && 'sel', r === true && 'hi', r === false && 'lo')}
                    d={pathD(e.pts)}
                    markerEnd={`url(#wf-ah-${on ? 's' : r ? 'h' : 'n'})`}
                  />
                  {!readOnly && (
                    <>
                      <path
                        className="wf-hit"
                        d={pathD(e.pts)}
                        onPointerDown={ev => ev.stopPropagation()}
                        onClick={() => {
                          setMenu(null);
                          setSel(s => (s === e.id ? null : e.id));
                        }}
                      >
                        <title>
                          {f.name(e.a)} → {f.name(e.b)}（点击选中，按住箭头可以改连）
                        </title>
                      </path>
                      <circle className="wf-grip" cx={e.pts[e.pts.length - 1][0]} cy={e.pts[e.pts.length - 1][1]} r="7" onPointerDown={startConnect(e.a, e.id, e.pts[0])} />
                    </>
                  )}
                </g>
              );
            })}
            {conn && <path className="wf-ghost" d={`M${conn.start[0]},${conn.start[1]} L${conn.end[0]},${conn.end[1]}`} markerEnd="url(#wf-ah-s)" />}
          </svg>

          {f.list.map(s => {
            const b = g.boxes[s.key];
            const n = counts[s.key] ?? 0;
            const warn = s.claim && !f.claimTarget(s.key);
            const lifted = lift?.key === s.key;
            const target = conn?.target === s.key;
            const exists = target && conn && f.hasEdge(conn.from, s.key) && edgeId(conn.from, s.key) !== conn.replace;
            const inner = (
              <>
                <i className="wf-dot" />
                <span className="wf-name">{s.name}</span>
                <b>{n}</b>
                {s.claim && (
                  <span className="wf-glyph claim" title="可领取：agent 从这里领取">
                    <Icon name="claim" />
                  </span>
                )}
                {s.lock && (
                  <span className="wf-glyph" title="锁定：只能由 agent 领取进入">
                    <Icon name="lock" />
                  </span>
                )}
                {s.final && (
                  <span className="wf-glyph" title="终点：进来算完成，只有人能移入移出">
                    <Icon name="check" />
                  </span>
                )}
                {warn && (
                  <span className="wf-glyph warn" title="没有连到锁定状态：agent 不能从这里领取">
                    <Icon name="alert" />
                  </span>
                )}
              </>
            );
            if (readOnly)
              return (
                <div key={s.key} className="wf-pos" style={{ left: b?.x ?? 0, top: b?.y ?? 0 }}>
                  <span
                    ref={el => {
                      chips.current[s.key] = el;
                    }}
                    className="wf-node"
                    style={{ ['--c' as string]: statusColor(s.color) }}
                    onPointerEnter={() => setHover(s.key)}
                    onPointerLeave={() => setHover(h => (h === s.key ? null : h))}
                  >
                    {inner}
                  </span>
                </div>
              );
            return (
              <div
                key={s.key}
                className="wf-pos"
                style={{
                  left: b?.x ?? 0,
                  top: b?.y ?? 0,
                  transform: lifted ? `translate(${lift!.dx}px, ${lift!.dy}px)` : undefined,
                  zIndex: lifted ? 5 : undefined,
                }}
              >
                <button
                  type="button"
                  ref={el => {
                    chips.current[s.key] = el;
                  }}
                  className={cx('wf-node', lifted && 'lift', menu === s.key && 'open', target && (exists ? 'tgt-exists' : 'tgt'))}
                  style={{ ['--c' as string]: statusColor(s.color) }}
                  onPointerDown={startLift(s.key)}
                  onPointerEnter={() => setHover(s.key)}
                  onPointerLeave={() => setHover(h => (h === s.key ? null : h))}
                  onClick={() => {
                    if (moved.current) return;
                    setSel(null);
                    setMenu(m => (m === s.key ? null : s.key));
                  }}
                  title="点击设置 · 拖动调整顺序 · 从右边缘的圆点拖出连线"
                >
                  {inner}
                  <span
                    className="wf-handle"
                    title="拖到另一个状态上，连一条线"
                    onPointerDown={e => b && startConnect(s.key, null, [b.x + b.w, b.y + CH / 2])(e)}
                    onClick={e => e.stopPropagation()}
                  />
                </button>
                {menu === s.key && (
                  <StatusMenu s={s} f={f} path={path} count={n} anchor={chips.current[s.key]} onClose={() => setMenu(null)} />
                )}
              </div>
            );
          })}

          {!readOnly && (
            <div className="wf-pos" style={{ left: g.add.x, top: g.add.y }}>
              {adding ? (
                <span className="wf-node add editing" ref={el => { addEl.current = el; }}>
                  <input
                    autoFocus
                    className="wf-input"
                    placeholder="状态名，回车添加"
                    maxLength={12}
                    onBlur={() => setAdding(false)}
                    onKeyDown={e => {
                      const t = e.target as HTMLInputElement;
                      if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                        const v = t.value.trim();
                        // 新状态的标识由服务端生成，拿到之前画不出来：输入框进入进行中（只读 + 转圈），不会重复添加
                        if (!v || t.readOnly) return;
                        t.readOnly = true;
                        t.parentElement?.classList.add('busy');
                        optimistic(null, () => api.createStatus(path, v))
                          .then(() => {
                            t.value = '';
                          })
                          .catch(err => {
                            toastError(err);
                            replay(t.parentElement, 'invalid');
                          })
                          .finally(() => {
                            t.readOnly = false;
                            t.parentElement?.classList.remove('busy');
                          });
                      }
                      if (e.key === 'Escape') {
                        e.stopPropagation();
                        setAdding(false);
                      }
                    }}
                  />
                </span>
              ) : (
                <button type="button" className="wf-node add" ref={el => { addEl.current = el; }} onClick={() => setAdding(true)}>
                  + 添加状态
                </button>
              )}
            </div>
          )}

          {selGeo && (
            <>
              <button
                type="button"
                className="wf-edge-del"
                style={{ left: midOf(selGeo.pts)[0], top: midOf(selGeo.pts)[1] }}
                onClick={() => removeEdge(selGeo.id)}
                title="删除这条连线（Delete）"
                aria-label={`删除连线 ${f.name(selGeo.a)} → ${f.name(selGeo.b)}`}
              >
                <Icon name="close" sw={2.2} />
              </button>
            </>
          )}
        </div>
      </div>
      {!readOnly && (
        <button
          type="button"
          ref={settingsEl}
          className={cx('wf-settings', settings && 'on')}
          aria-expanded={settings}
          title="这块白板自己的设置：状态块布局、保存顺序"
          onClick={() => setSettings(!settings)}
        >
          <Icon name="settings" />
          白板设置
        </button>
      )}
      {settings && <BoardSettings path={path} anchor={settingsEl.current} onClose={() => setSettings(false)} />}
    </div>
  );
}

function StatusMenu({
  s,
  f,
  path,
  count,
  anchor,
  onClose,
}: {
  s: Status;
  f: FlowQ;
  path: string;
  count: number;
  anchor: HTMLElement | null;
  onClose: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const busy = useBusy(flowKey(path));
  const close = useRef(onClose);
  close.current = onClose;
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useLayoutEffect(() => {
    const place = () => {
      const r = anchor?.getBoundingClientRect();
      if (r) setPos({ left: Math.min(r.left, innerWidth - 300), top: r.bottom + 8 });
    };
    place();
    addEventListener('resize', place);
    addEventListener('scroll', place, true);
    return () => {
      removeEventListener('resize', place);
      removeEventListener('scroll', place, true);
    };
  }, [anchor]);
  useEffect(() => {
    const down = (e: PointerEvent) => {
      if (!(e.target as HTMLElement).closest('.wf-menu, .wf-node')) close.current();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        close.current();
      }
    };
    addEventListener('pointerdown', down);
    addEventListener('keydown', key, true);
    return () => {
      removeEventListener('pointerdown', down);
      removeEventListener('keydown', key, true);
    };
  }, []);

  const patch = (p: Parameters<typeof api.updateStatus>[2]) =>
    run(list => list.map(x => (x.key === s.key ? { ...x, ...p } : x)), () => api.updateStatus(path, s.key, p));
  const rename = (v: string) => {
    const n = v.trim();
    if (n && n !== s.name) patch({ name: n });
  };
  const sw = (on: boolean, label: string, hint: string, set: () => void, locked?: string) => (
    <button type="button" className={cx('wf-sw', on && 'on')} onClick={locked ? undefined : set} aria-pressed={on} aria-disabled={!!locked}>
      <span className="wf-sw-track">
        <i />
      </span>
      <span className="wf-sw-text">
        <b>{label}</b>
        <span>{locked || hint}</span>
      </span>
    </button>
  );
  const lastClaim = s.claim && f.claimables.length === 1;
  const rest = f.list.find(x => x.claim && x.key !== s.key);

  if (!pos) return null;
  return createPortal(
    <div
      className={cx('wf-menu', busy && 'busy')}
      aria-busy={busy || undefined}
      style={{ ['--c' as string]: statusColor(s.color), position: 'fixed', left: pos.left, top: pos.top }}
      onPointerDown={e => e.stopPropagation()}
    >
      <input
        className="wf-menu-name"
        defaultValue={s.name}
        maxLength={12}
        autoFocus
        onFocus={e => e.target.select()}
        onBlur={e => rename(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            rename((e.target as HTMLInputElement).value);
            onClose();
          }
        }}
        aria-label="状态名"
      />
      <div className="wf-key">
        标识 <code>{s.key}</code>
        <span>CLI 里写标识或名称都可以</span>
      </div>
      <div className="wf-swatches">
        {STATUS_COLORS.map(c => (
          <button
            key={c}
            type="button"
            className={cx(c === s.color && 'on')}
            style={{ ['--sc' as string]: statusColor(c) }}
            onClick={() => c !== s.color && patch({ color: c })}
            aria-label="颜色"
          />
        ))}
      </div>
      <div className="wf-view">
        <span className="wf-view-label">卡片形态</span>
        <div className="wf-view-opts" role="radiogroup" aria-label="卡片形态">
          {VIEWS.map(([v, name, hint]) => (
            <button
              key={v}
              type="button"
              role="radio"
              aria-checked={viewOf(s) === v}
              className={cx(viewOf(s) === v && 'on')}
              title={hint}
              onClick={() => viewOf(s) !== v && patch({ view: v })}
            >
              <Icon name={v === 'card' ? 'viewCard' : v === 'line' ? 'viewLine' : 'viewTile'} />
              {name}
            </button>
          ))}
        </div>
      </div>
      <div className="wf-rules">
        {sw(
          s.claim,
          '可领取',
          'board claim 从这里领，领取后沿连线进入锁定状态；可以有多个',
          () => patch({ claim: !s.claim }),
          lastClaim ? '至少要有一个可领取的状态：先把另一个状态设为可领取' : undefined,
        )}
        {sw(s.lock, '锁定', '只能由 agent 领取进入；被持有期间只有持有者能移动', () => patch({ lock: !s.lock }))}
        {sw(s.final, '终点', '进来算完成：只有人能移入移出', () => patch({ final: !s.final }))}
      </div>
      <div className="wf-menu-foot">
        {lastClaim ? (
          <span className="wf-note">最后一个可领取的状态不能删除</span>
        ) : confirming ? (
          <button
            type="button"
            className="wf-del sure"
            onClick={() => {
              onClose();
              // 状态和指向它的连线当帧消失；里面的卡片由服务端移走，刷新后出现在新位置
              run(list => list.filter(x => x.key !== s.key).map(x => (x.to.includes(s.key) ? { ...x, to: x.to.filter(t => t !== s.key) } : x)), () => api.deleteStatus(path, s.key));
            }}
          >
            确认删除{count ? `，${count} 张卡片移到「${(s.key === f.home?.key ? rest : f.home)?.name}」` : ''}，相关连线一起删除
          </button>
        ) : (
          <button type="button" className="wf-del" onClick={() => setConfirming(true)}>
            删除状态
          </button>
        )}
      </div>
    </div>,
    document.body,
  );
}
