/* ==========================================================================
   分工（Agent tab → 自动领取，PRD §9.1「分工」）：规则、角色、机器放在一页
   - 最上面一行是参与自动领取的白板（点一下参与 / 不参与，和白板设置里是同一个值）
   - 流向图：规则 → 角色 → 机器三栏，用线连起来。实线是现在就能接（在线、自动领、在接单时段里、有空位），虚线是能跑但现在接不了；
     悬停一条规则亮它会走的线，点一个角色在下面看它的详情（领哪些卡、在哪些机器上能跑、现在在跑和排着的、编辑）
   - 规则在弹出框里改（条件、标签、交给哪个角色），先后用上下箭头调；规则都没命中的卡不自动领，可以手动派发
   - 角色在弹出框里新建、改、删（ProfileForm）
   写操作：规则、白板开关是设置项（saveSetting，当帧生效，保存状态画在旁边）；角色见 ProfileForm
   ========================================================================== */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import { openBoard, openCard } from '../../lib/actions';
import { hostsOf } from '../../lib/hosts';
import { forecast, hostsFor, profilesOf, recipe, runningOf, short, slotOf, type Fc } from '../../lib/profiles';
import { claimable, serverMinute } from '../../lib/sched';
import type { ClaimRule, Profile, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { boardAutoPatch, saveSetting, useData } from '../../store/data';
import { setTab } from '../../store/ui';
import { Segmented, Select, Sync, Tip, useNow, useSync } from '../bits';
import { OsGlyph } from '../hosts/bits';
import { LabelSet } from '../hosts/LabelSet';
import { Icon } from '../Icon';
import { NameChips, PMark, ProfileForm, RuleView, Slots, agentName, colorOf, openProfile, useAllLabels, useProfileBusy, useProviders } from './bits';

type Rule = Pick<ClaimRule, 'op' | 'labels' | 'profile'>;
const plain = (r: ClaimRule): Rule => ({ op: r.op, labels: r.op === 'none' ? [] : r.labels, profile: r.profile });

/** 规则弹出框里能选的条件（标签正好是、只带这几个里的这两种旧条件存着的照样显示、照样能改别的） */
const OPS: [ClaimRule['op'], string][] = [
  ['in', '带'],
  ['all', '同时带'],
  ['not_in', '不带'],
  ['none', '没有标签'],
];

type Edge = { from: string; to: string; dash: boolean; color: string; prof: string; rule?: number };

export function Division() {
  const ws = useData(s => s.ws);
  const board = useData(s => s.board);
  const now = useNow(30_000);
  const { sync, track } = useSync();
  useProviders();
  const hosts = hostsOf(ws);
  const profiles = profilesOf(ws);
  const rules = ws?.sched?.rules ?? [];
  const latest = useRef<ClaimRule[]>([]);
  latest.current = rules;
  const [sel, setSel] = useState<string | null>(null);
  const [hover, setHover] = useState<{ prof?: string; rule?: number } | null>(null);
  const [editing, setEditing] = useState<number | 'new' | null>(null);
  const map = useRef<HTMLDivElement>(null);
  const [paths, setPaths] = useState<(Edge & { d: string; k: string })[]>([]);
  const minute = serverMinute(ws, now);
  const fc = useMemo(() => forecast(ws, board, now), [ws, board, now]);

  const edges = useMemo(() => {
    const e: Edge[] = [];
    rules.forEach((r, i) => {
      const p = profiles.find(x => x.id === r.profile);
      if (p) e.push({ from: `r${i}`, to: `p${p.id}`, dash: false, color: colorOf(p), prof: p.id, rule: i });
    });
    for (const p of profiles) {
      for (const x of hostsFor(ws, p.id, minute, true)) if (x.can) e.push({ from: `p${p.id}`, to: `h${x.host.id}`, dash: !x.ok, color: colorOf(p), prof: p.id });
    }
    return e;
  }, [rules, profiles, ws, minute]);

  useLayoutEffect(() => {
    const el = map.current;
    if (!el) return;
    const draw = () => {
      if (el.offsetWidth < 760) return setPaths([]);
      const box = el.getBoundingClientRect();
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

  if (!ws || !ws.sched) return null;
  const saveRules = (next: Rule[]) => {
    track('rules', saveSetting('sched:rules', { ws: w => (w.sched ? { ...w, sched: { ...w.sched, rules: next.map(r => ({ ...r, active: true })) } } : w) }, () => api.updateSched({ rules: next })));
  };
  const toggleBoard = (path: string, on: boolean) => track(`board:${path}`, saveSetting(`sched:board:${path}`, boardAutoPatch(path, on), () => api.updateBoard(path, { auto: on })));
  const move = (i: number, d: number) => {
    const next = latest.current.map(plain);
    const [r] = next.splice(i, 1);
    next.splice(i + d, 0, r);
    saveRules(next);
  };
  const boards = ws.projects.flatMap(p => p.boards);
  const pick = profiles.find(x => x.id === sel) ?? profiles[0];
  // 高亮：悬停的规则 → 它的角色 → 机器；否则悬停 / 选中的角色
  const focusProf = hover?.rule != null ? rules[hover.rule]?.profile : (hover?.prof ?? pick?.id);
  const lit = (e: Edge) => (hover?.rule != null ? (e.rule != null ? e.rule === hover.rule : e.prof === focusProf) : e.prof === focusProf);
  const counts = (i: number) => [...fc.values()].filter(f => f.route?.rule === i).length;
  const queued = (p: string) => (ws.dispatches ?? []).filter(d => d.state === 'queued' && d.profile === p).length + [...fc.values()].filter(f => f.route?.profile === p && f.kind === 'wait').length;

  return (
    <section className="sc pfa">
      <div className="sc-head">
        <div>
          <h2>分工</h2>
          <p>
            每张卡按规则交给一个角色；角色定下用哪个 agent、哪个模型、多大强度；能跑这个角色的机器里谁空给谁。规则都没命中的卡不自动领，可以手动派发。实线是现在就能接，虚线是能跑但现在接不了。时间按服务器时间
            {ws.sched.tz ? `（${ws.sched.tz}）` : ''}，每台机器什么时候接单在「机器 → 接单」里。
          </p>
        </div>
        <div className="sc-acts">
          <span className="hp-ctl">
            <Sync s={sync.rules} />
            <button type="button" className="btn sm" disabled={!profiles.length} onClick={() => setEditing('new')}>
              <Icon name="plus" />
              加一条规则
            </button>
          </span>
          <button type="button" className="btn sm" onClick={() => openProfile(null)}>
            <Icon name="plus" />
            新建角色
          </button>
        </div>
      </div>

      <div className="sd-boards">
        <span className="sd-k">自动领取的白板</span>
        {boards.map(b => {
          const on = b.auto !== false;
          return (
            <Tip key={b.path} text={on ? `${b.name}：点一下不再自动领取（手动派发照常）` : `${b.name}：点一下参与自动领取`}>
              <button type="button" className={cx('sd-bchip', on && 'on', sync[`board:${b.path}`] === 'failed' && 'hp-rollback')} aria-pressed={on} onClick={() => toggleBoard(b.path, !on)}>
                <i />
                {b.path}
                <small>{claimable(b)}</small>
              </button>
            </Tip>
          );
        })}
      </div>

      {profiles.length === 0 ? (
        <p className="hp-empty sd-empty">还没有角色：点「新建角色」定下用哪个 agent、哪个模型，再加规则把卡交给它。</p>
      ) : (
        <div className={cx('pfa-map', sync.rules === 'failed' && 'hp-rollback')} ref={map}>
          <svg className="pfa-lines" aria-hidden="true">
            {paths.map(x => (
              <path key={x.k} d={x.d} className={cx(x.dash && 'dash', lit(x) ? 'lit' : 'dim')} style={{ ['--c' as string]: x.color }} />
            ))}
          </svg>
          <div className="pfa-col">
            <div className="pfa-h">
              规则 <small>从上往下，第一条命中的生效</small>
            </div>
            {rules.length === 0 && <p className="pf-dim pfa-none">还没有规则：卡片都不自动领</p>}
            {rules.map((r, i) => (
              <div key={`${i}-${r.op}-${r.labels.join(',')}`} data-k={`r${i}`} className={cx('pfa-rule', hover?.rule === i && 'hover', r.active === false && 'off')} onMouseEnter={() => setHover({ rule: i })} onMouseLeave={() => setHover(null)}>
                <span className="pfa-no">{i + 1}</span>
                <span className="pfa-rt">
                  <RuleView r={r} />
                </span>
                {counts(i) > 0 && <small className="pfa-n">{counts(i)} 张</small>}
                <span className="pf-ra">
                  <button type="button" className="icon-btn sm" aria-label="改这条规则" onClick={() => setEditing(i)}>
                    <Icon name="edit" />
                  </button>
                  <button type="button" className="icon-btn sm" aria-label="提前" disabled={i === 0} onClick={() => move(i, -1)}>
                    <Icon name="chevUp" />
                  </button>
                  <button type="button" className="icon-btn sm" aria-label="推后" disabled={i === rules.length - 1} onClick={() => move(i, 1)}>
                    <Icon name="chevDown" />
                  </button>
                </span>
              </div>
            ))}
          </div>

          <div className="pfa-col">
            <div className="pfa-h">
              角色 <small>agent · 模型 · 强度</small>
            </div>
            {profiles.map(x => {
              const run = runningOf(ws, x.id).length;
              const wait = queued(x.id);
              return (
                <button
                  key={x.id}
                  type="button"
                  data-k={`p${x.id}`}
                  className={cx('pfa-prof', pick?.id === x.id && 'on', focusProf === x.id && 'lit')}
                  style={{ ['--c' as string]: colorOf(x) }}
                  onClick={() => setSel(x.id)}
                  onMouseEnter={() => setHover({ prof: x.id })}
                  onMouseLeave={() => setHover(null)}
                >
                  <PMark p={x} />
                  <span className="pfa-pt">
                    <b>{x.name}</b>
                    <small>{short(x)}</small>
                  </span>
                  <span className="pfa-load">
                    {run > 0 && <em>在跑 {run}</em>}
                    {wait > 0 && <em className="wait">等 {wait}</em>}
                  </span>
                </button>
              );
            })}
          </div>

          <div className="pfa-col">
            <div className="pfa-h">
              机器 <small>会话位</small>
            </div>
            {hosts.length === 0 && <p className="pf-dim pfa-none">还没有机器</p>}
            {hosts.map(h => (
              <div key={h.id} data-k={`h${h.id}`} className={cx('pfa-host', !h.online && 'off')}>
                <OsGlyph os={h.os} small />
                <b>{h.name}</b>
                <Slots h={h} ws={ws} />
                <small>{slotOf(ws, h, minute, true).why}</small>
              </div>
            ))}
          </div>
        </div>
      )}

      {pick && <ProfileDetail p={pick} fc={fc} ws={ws} minute={minute} boardPath={board?.path} />}

      {editing != null && (
        <RuleDialog
          r={editing === 'new' ? { op: 'in', labels: [], profile: pick?.id ?? profiles[0]?.id ?? '' } : plain(rules[editing])}
          no={editing === 'new' ? undefined : editing + 1}
          profiles={profiles}
          onCancel={() => setEditing(null)}
          onDelete={editing === 'new' ? undefined : () => (saveRules(latest.current.filter((_, j) => j !== editing).map(plain)), setEditing(null))}
          onSave={x => {
            const cur = latest.current.map(plain);
            saveRules(editing === 'new' ? [...cur, x] : cur.map((r, j) => (j === editing ? x : r)));
            setEditing(null);
          }}
        />
      )}
      <ProfileForm />
    </section>
  );
}

/** 去看一张卡：切到它的白板，打开详情 */
const goCard = (board: string, id: number) => {
  setTab('board');
  openBoard(board);
  setTimeout(() => openCard(id), 150);
};

/** 分工页下面：选中的角色。一行抬头（logo、名字、配方、编辑），下面四行：领卡、机器、现在；左对齐，颜色只用在状态点上 */
function ProfileDetail({ p, fc, ws, minute, boardPath }: { p: Profile; fc: Map<number, Fc>; ws: Workspace; minute: number; boardPath?: string }) {
  const busy = useProfileBusy(p.id);
  const rules = (ws.sched?.rules ?? []).map((r, i) => ({ r, i })).filter(x => x.r.profile === p.id);
  const running = runningOf(ws, p.id);
  const queued = (ws.dispatches ?? []).filter(d => d.state === 'queued' && d.profile === p.id);
  const waiting = [...fc.values()].filter(f => f.route?.profile === p.id && (f.kind === 'wait' || f.kind === 'start'));
  return (
    <div className={cx('pfq', busy && 'is-saving')}>
      <header className="pfq-head">
        <PMark p={p} />
        <b className="pfq-name">{p.name}</b>
        <small className="pfq-recipe">{recipe(p, agentName)}</small>
        <button type="button" className="btn sm ghost pfq-edit" disabled={busy} onClick={() => openProfile(p)}>
          <Icon name="edit" />
          编辑
        </button>
      </header>
      {p.note && <p className="pfq-note">{p.note}</p>}
      <dl className="pfq-dl">
        <dt>领卡</dt>
        <dd>
          {rules.length ? (
            rules.map(x => (
              <span key={x.i} className="pfq-rule">
                <i>{x.i + 1}</i>
                <RuleView r={x.r} />
              </span>
            ))
          ) : (
            <em>没有规则交给它，只能手动派发</em>
          )}
        </dd>
        <dt>机器</dt>
        <dd>
          {hostsFor(ws, p.id, minute, true).map(x => (
            <span key={x.host.id} className={cx('pfq-host', `t-${x.tone}`, !x.can && 'cant')} title={`${x.host.name}：${x.why}`}>
              <i />
              <b>{x.host.name}</b>
              {x.why}
            </span>
          ))}
        </dd>
        <dt>现在</dt>
        <dd className="pfq-now">
          {running.map(d => (
            <button key={d.id} type="button" className="pfq-row run" onClick={() => goCard(d.board, d.card)}>
              <span className="pfq-tag">
                <i />
                在跑
              </span>
              <span className="pfq-t">
                <b>
                  {d.board}#{d.card}
                </b>{' '}
                {d.title}
              </span>
              <small>{d.host}</small>
            </button>
          ))}
          {queued.map(d => (
            <button key={d.id} type="button" className="pfq-row k-wait" onClick={() => goCard(d.board, d.card)}>
              <span className="pfq-tag">已派发</span>
              <span className="pfq-t">
                <b>
                  {d.board}#{d.card}
                </b>{' '}
                {d.title}
              </span>
              <small>{d.host || '任意机器'}</small>
            </button>
          ))}
          {boardPath &&
            waiting.map(f => (
              <button key={f.card} type="button" className={cx('pfq-row', `k-${f.kind}`)} onClick={() => goCard(boardPath, f.card)}>
                <span className="pfq-tag">{f.kind === 'start' ? '马上开始' : `排第 ${f.pos}`}</span>
                <span className="pfq-t">
                  <b>#{f.card}</b> {f.title}
                </span>
                <small>{f.kind === 'start' ? f.host : (f.soon ?? '').replace('空出来就开始', '空出来')}</small>
              </button>
            ))}
          {!running.length && !queued.length && !waiting.length && <em>没有在跑、也没有在等的卡{boardPath ? `（${boardPath}）` : ''}</em>}
        </dd>
      </dl>
    </div>
  );
}

/** 改 / 加一条规则：弹出框。高度固定：「没有标签」时标签框留着、变灰；标签框固定两行高。切到「没有标签」再切回来，选过的标签还在（保存时才清掉） */
function RuleDialog({ r, no, profiles, onSave, onCancel, onDelete }: { r: Rule; no?: number; profiles: Profile[]; onSave: (r: Rule) => void; onCancel: () => void; onDelete?: () => void }) {
  const all = useAllLabels();
  const [d, setD] = useState(r);
  const none = d.op === 'none';
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === 'Escape' && !e.defaultPrevented && onCancel();
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [onCancel]);
  const ops = OPS.some(([v]) => v === d.op) ? OPS : [...OPS, [d.op, r.op === 'equal' ? '标签正好是' : '只带这几个里的'] as [ClaimRule['op'], string]];
  return createPortal(
    <div className="dialog-wrap on" onClick={e => e.target === e.currentTarget && onCancel()}>
      <div className="dialog pf-form pf-rd" role="dialog" aria-modal="true" aria-label={no ? `规则 ${no}` : '加一条规则'}>
        <h3>{no ? `规则 ${no}` : '加一条规则'}</h3>
        <div className="pf-f">
          <span>条件</span>
          <Segmented<ClaimRule['op']> value={d.op} options={ops} onChange={v => setD({ ...d, op: v })} />
        </div>
        <div className="pf-f">
          <span>标签</span>
          <div className={cx('pf-rd-ls', none && 'off')}>{none ? <em>卡片没有任何标签时命中，不用选</em> : <LabelSet value={d.labels} labels={all} onChange={v => setD({ ...d, labels: v })} />}</div>
        </div>
        <label className="pf-f">
          <span>交给</span>
          <Select value={d.profile} options={profiles.map(p => ({ value: p.id, label: `${p.name} · ${short(p)}` }))} aria-label="交给哪个角色" onChange={v => setD({ ...d, profile: v })} />
        </label>
        <div className="dialog-actions pf-form-acts">
          {onDelete && (
            <button type="button" className="btn sm danger" onClick={onDelete}>
              <Icon name="trash" />
              删除
            </button>
          )}
          <span className="hp-sp" />
          <button className="btn" type="button" onClick={onCancel}>
            取消
          </button>
          <button className="btn brand" type="button" disabled={(!none && !d.labels.length) || !d.profile} onClick={() => onSave(none ? { ...d, labels: [] } : d)}>
            保存
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export { NameChips };
