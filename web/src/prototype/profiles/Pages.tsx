/* ==========================================================================
   原型（用完即弃）：Agent → 自动领取 → 分工，三个变体（?pg=A|B|C；0 = 现在的「领取规则」）
   A 流向图：规则 → 工种 → 机器三栏，用线连起来；点一个工种，下面写它的配方、在哪能跑、现在在做什么、成绩
   B 工种卡片：一个工种一张卡（领哪些卡、在哪跑、现在、近 30 天），和现在「按 agent 分卡」最像
   C 路由表 + 试一张：左边规则表、工种表；右边挑一张卡（或自己拼一组标签），一步步写它会怎么走
   ========================================================================== */
import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { agentMark } from '../../lib/hosts';
import { cx } from '../../lib/util';
import { useData } from '../../store/data';
import { openCard, openBoard } from '../../lib/actions';
import { setTab } from '../../store/ui';
import { Select } from '../../components/bits';
import { OsGlyph } from '../../components/hosts/bits';
import { Icon } from '../../components/Icon';
import { HostWhyChip, PChip, PMark, StatsLine, openProfile, useFc } from './bits';
import {
  OPS,
  canRun,
  hostsFor,
  loadOf,
  moveRule,
  pickHost,
  recipe,
  routeOf,
  ruleHit,
  ruleText,
  setFallback,
  setRules,
  short,
  slot,
  soonest,
  toggleOnly,
  useModel,
  type Fc,
  type PHost,
  type Profile,
  type Rule,
} from './model';
import { usePg } from './proto';

export function ProfilesPage({ current }: { current: ReactNode }) {
  const pg = usePg();
  if (pg === '0') return <>{current}</>;
  return pg === 'A' ? <VariantA /> : pg === 'B' ? <VariantB /> : <VariantC />;
}

/* ------------------------------ 共用 ------------------------------ */

export const colorOf = (p: Profile | undefined) => (p?.agent ? agentMark(p.agent).color : 'var(--text-3)');

/** 一张卡的推算写成一句 */
export function verdict(f: Fc | undefined): { text: string; tone: 'ok' | 'wait' | 'me' | 'none' } {
  if (!f) return { text: '', tone: 'none' };
  if (f.kind === 'start') return { text: `马上在 ${f.host} 开始`, tone: 'ok' };
  if (f.kind === 'wait') return { text: `排第 ${f.pos}：${f.soon}`, tone: 'wait' };
  if (f.kind === 'me') return { text: '等你开会话', tone: 'me' };
  return { text: '不自动领：没有命中规则', tone: 'none' };
}

function useLabelNames() {
  const ws = useData(s => s.ws);
  return useMemo(() => {
    const seen = new Set<string>();
    return (ws?.projects ?? []).flatMap(p => p.labels.map(l => l.name)).filter(n => !seen.has(n.toLowerCase()) && !!seen.add(n.toLowerCase()));
  }, [ws?.projects]);
}

const goCard = (id: number) => {
  setTab('board');
  openBoard('oss/main');
  setTimeout(() => openCard(id), 150);
};

/** 改一条规则：条件、标签、（可选）交给哪个工种 */
function RuleEditor({ r, onSave, onCancel, withTarget }: { r: Rule; onSave: (r: Rule) => void; onCancel: () => void; withTarget?: boolean }) {
  const labels = useLabelNames();
  const profiles = useModel(s => s.profiles);
  const [d, setD] = useState(r);
  return (
    <div className="pf-re">
      <Select className="pf-re-op" value={d.op} options={OPS} aria-label="条件" onChange={v => setD({ ...d, op: v as Rule['op'], labels: v === 'none' ? [] : d.labels })} />
      {d.op !== 'none' && (
        <div className="pf-re-ls">
          {labels.map(l => (
            <button key={l} type="button" className={cx('pf-re-l', d.labels.includes(l) && 'on')} onClick={() => setD({ ...d, labels: d.labels.includes(l) ? d.labels.filter(x => x !== l) : [...d.labels, l] })}>
              {l}
            </button>
          ))}
        </div>
      )}
      {withTarget && <Select className="pf-re-to" value={d.to} options={profiles.map(p => ({ value: p.id, label: `→ ${p.name}` }))} aria-label="交给哪个工种" onChange={v => setD({ ...d, to: v })} />}
      <span className="pf-re-acts">
        <button type="button" className="btn sm" onClick={onCancel}>
          取消
        </button>
        <button type="button" className="btn sm brand" disabled={d.op !== 'none' && !d.labels.length} onClick={() => onSave(d)}>
          好
        </button>
      </span>
    </div>
  );
}

const saveRule = (i: number, r: Rule) => setRules(useModel.getState().rules.map((x, j) => (j === i ? r : x)));
const dropRule = (i: number) => setRules(useModel.getState().rules.filter((_, j) => j !== i));
const addRule = (r: Rule) => setRules([...useModel.getState().rules, r]);

function RuleActs({ i, n, onEdit }: { i: number; n: number; onEdit?: () => void }) {
  return (
    <span className="pf-ra">
      {onEdit && (
        <button type="button" className="icon-btn sm" aria-label="改这条规则" onClick={onEdit}>
          <Icon name="edit" />
        </button>
      )}
      <button type="button" className="icon-btn sm" aria-label="提前" disabled={i === 0} onClick={() => moveRule(i, -1)}>
        <Icon name="chevUp" />
      </button>
      <button type="button" className="icon-btn sm" aria-label="推后" disabled={i === n - 1} onClick={() => moveRule(i, 1)}>
        <Icon name="chevDown" />
      </button>
      <button type="button" className="icon-btn sm" aria-label="删掉这条规则" onClick={() => dropRule(i)}>
        <Icon name="trash" />
      </button>
    </span>
  );
}

function FallbackSelect() {
  const { profiles, fallback } = useModel();
  return (
    <Select
      className="pf-fb"
      value={fallback ?? ''}
      options={[{ value: '', label: '不自动领' }, ...profiles.map(p => ({ value: p.id, label: `→ ${p.name}` }))]}
      aria-label="其余卡片交给谁"
      onChange={v => setFallback(v || null)}
    />
  );
}

/** 机器的会话位：在跑的用工种的颜色填上，空位空着；离线、时段外画斜纹 */
function Slots({ h }: { h: PHost }) {
  const profiles = useModel(s => s.profiles);
  return (
    <span className={cx('pf-slots', !h.online && 'off')}>
      {Array.from({ length: h.max }, (_, i) => {
        const r = h.running[i];
        const p = r && profiles.find(x => x.id === r.profile);
        return <i key={i} className={cx(r && 'on')} style={r ? { ['--c' as string]: colorOf(p) } : undefined} title={r ? `${p?.name ?? r.profile} · ${r.ref} ${r.title} · ${r.min} 分钟` : '空位'} />;
      })}
    </span>
  );
}

/** 机器栏里的一句：按自动领取算（离线、时段外、满、空几位） */
const hostState = (h: PHost) => slot(h, 0, true).why.replace('开始接单', '起');

const PageHead = ({ text, children }: { text: string; children?: ReactNode }) => (
  <div className="sc-head">
    <div>
      <h2>分工</h2>
      <p>{text}</p>
    </div>
    <div className="sc-acts">{children}</div>
  </div>
);

/* ------------------------------ A 流向图 ------------------------------ */

type Edge = { from: string; to: string; dash: boolean; color: string; prof: string; rule?: number };

function VariantA() {
  const { profiles, rules, fallback, hosts } = useModel();
  const fc = useFc();
  const [sel, setSel] = useState('bulk');
  const [hover, setHover] = useState<{ prof?: string; rule?: number } | null>(null);
  const [editing, setEditing] = useState<number | 'new' | null>(null);
  const map = useRef<HTMLDivElement>(null);
  const [paths, setPaths] = useState<(Edge & { d: string; k: string })[]>([]);
  const counts = (i: number) => [...fc.values()].filter(f => f.route?.rule === i).length;
  const unmatched = [...fc.values()].filter(f => f.kind === 'none');

  const edges = useMemo(() => {
    const e: Edge[] = [];
    rules.forEach((r, i) => {
      const p = profiles.find(x => x.id === r.to);
      if (p) e.push({ from: `r${i}`, to: `p${p.id}`, dash: false, color: colorOf(p), prof: p.id, rule: i });
    });
    const fb = profiles.find(x => x.id === fallback);
    e.push({ from: 'rf', to: fb ? `p${fb.id}` : 'pnone', dash: true, color: fb ? colorOf(fb) : 'var(--text-4)', prof: fb?.id ?? 'none', rule: -1 });
    for (const p of profiles) {
      if (!p.agent) {
        e.push({ from: `p${p.id}`, to: 'hme', dash: false, color: 'var(--text-3)', prof: p.id });
        continue;
      }
      for (const x of hostsFor(p, hosts, true)) if (x.can) e.push({ from: `p${p.id}`, to: `h${x.name}`, dash: !x.ok, color: colorOf(p), prof: p.id });
    }
    return e;
  }, [rules, profiles, fallback, hosts]);

  useLayoutEffect(() => {
    const el = map.current;
    if (!el) return;
    const draw = () => {
      if (el.offsetWidth < 760) return setPaths([]);
      const box = el.getBoundingClientRect();
      const at = (k: string) => el.querySelector<HTMLElement>(`[data-k="${k}"]`)?.getBoundingClientRect();
      setPaths(
        edges.flatMap((e, i) => {
          const a = at(e.from);
          const b = at(e.to);
          if (!a || !b) return [];
          const x1 = a.right - box.left;
          const y1 = a.top + a.height / 2 - box.top;
          const x2 = b.left - box.left;
          const y2 = b.top + b.height / 2 - box.top;
          const dx = Math.max(20, (x2 - x1) * 0.5);
          return [{ ...e, k: `${e.from}>${e.to}#${i}`, d: `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}` }];
        }),
      );
    };
    draw();
    const ro = new ResizeObserver(draw);
    ro.observe(el);
    return () => ro.disconnect();
  }, [edges, editing]);

  // 高亮：悬停的规则 → 它的工种 → 机器；否则悬停 / 选中的工种
  const focusProf = hover?.rule != null ? (hover.rule === -1 ? (fallback ?? 'none') : rules[hover.rule]?.to) : (hover?.prof ?? sel);
  const lit = (e: Edge) => (hover?.rule != null ? (e.rule != null ? e.rule === hover.rule : e.prof === focusProf) : e.prof === focusProf);
  const p = profiles.find(x => x.id === sel) ?? profiles[0];

  return (
    <section className="sc pfa">
      <PageHead text="每张卡按规则交给一个工种；工种定下用哪个 agent、哪个模型、多大强度；能跑这个工种的机器里谁空给谁。实线是现在就能接，虚线是能跑但现在接不了。">
        <button type="button" className="btn sm" onClick={() => setEditing('new')}>
          <Icon name="plus" />
          加一条规则
        </button>
        <button type="button" className="btn sm" onClick={() => openProfile(null)}>
          <Icon name="plus" />
          新建工种
        </button>
      </PageHead>

      <div className="pfa-map" ref={map}>
        <svg className="pfa-lines" aria-hidden="true">
          {paths.map(x => (
            <path key={x.k} d={x.d} className={cx(x.dash && 'dash', lit(x) ? 'lit' : 'dim')} style={{ ['--c' as string]: x.color }} />
          ))}
        </svg>
        <div className="pfa-col">
          <div className="pfa-h">
            规则 <small>从上往下，第一条命中的生效</small>
          </div>
          {rules.map((r, i) =>
            editing === i ? (
              <RuleEditor key={i} r={r} withTarget onCancel={() => setEditing(null)} onSave={x => (saveRule(i, x), setEditing(null))} />
            ) : (
              <div key={i} data-k={`r${i}`} className={cx('pfa-rule', hover?.rule === i && 'hover')} onMouseEnter={() => setHover({ rule: i })} onMouseLeave={() => setHover(null)}>
                <span className="pfa-no">{i + 1}</span>
                <span className="pfa-rt">{ruleText(r)}</span>
                {counts(i) > 0 && <small className="pfa-n">{counts(i)} 张</small>}
                <RuleActs i={i} n={rules.length} onEdit={() => setEditing(i)} />
              </div>
            ),
          )}
          {editing === 'new' && (
            <RuleEditor r={{ op: 'in', labels: [], to: sel }} withTarget onCancel={() => setEditing(null)} onSave={x => (addRule(x), setEditing(null))} />
          )}
          <div data-k="rf" className={cx('pfa-rule fb', hover?.rule === -1 && 'hover')} onMouseEnter={() => setHover({ rule: -1 })} onMouseLeave={() => setHover(null)}>
            <span className="pfa-no">·</span>
            <span className="pfa-rt">其余卡片</span>
            {unmatched.length > 0 && <small className="pfa-n">{unmatched.length} 张</small>}
            <FallbackSelect />
          </div>
        </div>

        <div className="pfa-col">
          <div className="pfa-h">
            工种 <small>agent · 模型 · 强度</small>
          </div>
          {profiles.map(x => {
            const l = loadOf(x, hosts, fc);
            return (
              <button
                key={x.id}
                type="button"
                data-k={`p${x.id}`}
                className={cx('pfa-prof', sel === x.id && 'on', focusProf === x.id && 'lit')}
                style={{ ['--c' as string]: colorOf(x) }}
                onClick={() => setSel(x.id)}
                onMouseEnter={() => setHover({ prof: x.id })}
                onMouseLeave={() => setHover(null)}
              >
                <PMark p={x} />
                <span className="pfa-pt">
                  <b>{x.name}</b>
                  <small>{x.agent ? short(x) : '不派给机器'}</small>
                </span>
                <span className="pfa-load">
                  {l.running.length > 0 && <em>在跑 {l.running.length}</em>}
                  {l.waiting.length + l.starting.length > 0 && <em className={cx(l.waiting.length > 0 && 'wait')}>等 {l.waiting.length + l.starting.length}</em>}
                </span>
              </button>
            );
          })}
          <div data-k="pnone" className={cx('pfa-prof none', focusProf === 'none' && 'lit')}>
            <span className="pf-mark">–</span>
            <span className="pfa-pt">
              <b>不自动领</b>
              <small>可以手动派发</small>
            </span>
          </div>
        </div>

        <div className="pfa-col">
          <div className="pfa-h">
            机器 <small>会话位</small>
          </div>
          {hosts.map(h => (
            <div key={h.name} data-k={`h${h.name}`} className={cx('pfa-host', !h.online && 'off')}>
              <OsGlyph os={h.os} small />
              <b>{h.name}</b>
              <Slots h={h} />
              <small>{hostState(h)}</small>
            </div>
          ))}
          <div data-k="hme" className="pfa-host me">
            <span className="pf-mark sm me">我</span>
            <b>你自己开会话</b>
          </div>
        </div>
      </div>

      {p && <ProfileDetail p={p} fc={fc} />}
    </section>
  );
}

/** A 下面：选中的工种 */
function ProfileDetail({ p, fc }: { p: Profile; fc: Map<number, Fc> }) {
  const { hosts, rules } = useModel();
  const l = loadOf(p, hosts, fc);
  const mine = rules.map((r, i) => ({ r, i })).filter(x => x.r.to === p.id);
  return (
    <div className="pfa-detail" style={{ ['--c' as string]: colorOf(p) }}>
      <header>
        <PMark p={p} size="lg" />
        <div>
          <h3>{p.name}</h3>
          <p>{recipe(p)}</p>
        </div>
        <button type="button" className="btn sm" onClick={() => openProfile(p)}>
          <Icon name="edit" />
          改工种
        </button>
      </header>
      {p.note && <p className="pf-note">附加说明：{p.note}</p>}
      <dl className="pf-dl">
        <dt>领这些卡</dt>
        <dd>{mine.length ? mine.map(x => <span key={x.i} className="pf-rchip"><b>{x.i + 1}</b>{ruleText(x.r)}</span>) : <em>没有规则交给它：只能手动派发</em>}</dd>
        {p.agent && (
          <>
            <dt>在哪跑</dt>
            <dd>
              {hostsFor(p, hosts, true).map(x => (
                <HostWhyChip key={x.name} x={x} />
              ))}
            </dd>
          </>
        )}
        <dt>现在</dt>
        <dd className="pf-now">
          {l.running.map(r => (
            <span key={r.ref} className="pf-run">
              <i />
              {r.host} · {r.ref} {r.title} · {r.min} 分钟
            </span>
          ))}
          {[...l.starting, ...l.waiting].map(f => (
            <button key={f.card} type="button" className={cx('pf-wait', `t-${verdict(f).tone}`)} onClick={() => goCard(f.card)}>
              <b>#{f.card}</b> {f.title}
              <small>{verdict(f).text}</small>
            </button>
          ))}
          {!l.running.length && !l.waiting.length && !l.starting.length && <em>没有在跑、也没有在等的卡</em>}
        </dd>
        <dt>近 30 天</dt>
        <dd>
          <StatsLine s={p.stats} me={!p.agent} />
          <small className="pf-fake">（假数据）</small>
        </dd>
      </dl>
    </div>
  );
}

/* ------------------------------ B 工种卡片 ------------------------------ */

function VariantB() {
  const { profiles, rules, hosts } = useModel();
  const fc = useFc();
  const [editing, setEditing] = useState<{ i: number | 'new'; to: string } | null>(null);
  const unmatched = [...fc.values()].filter(f => f.kind === 'none');
  const all = hosts.map(h => h.name);
  return (
    <section className="sc pfb">
      <PageHead text="一个工种一张卡：领哪些卡、在哪些机器上跑、现在在做什么、做得怎么样。规则前面的数字是全局的先后，一张卡用第一条命中的。">
        <button type="button" className="btn sm" onClick={() => openProfile(null)}>
          <Icon name="plus" />
          新建工种
        </button>
      </PageHead>
      <div className="pfb-list">
        {profiles.map(p => {
          const mine = rules.map((r, i) => ({ r, i })).filter(x => x.r.to === p.id);
          const l = loadOf(p, hosts, fc);
          const hw = hostsFor(p, hosts, true);
          return (
            <article key={p.id} className={cx('pfb-card', !p.agent && 'me')} style={{ ['--c' as string]: colorOf(p) }}>
              <header>
                <PMark p={p} size="lg" />
                <div className="pfb-t">
                  <h3>{p.name}</h3>
                  <p>{recipe(p)}</p>
                </div>
                <span className="pfb-now">
                  {l.running.length > 0 && <em>在跑 {l.running.length}</em>}
                  {l.waiting.length + l.starting.length > 0 && <em className="wait">等 {l.waiting.length + l.starting.length}</em>}
                </span>
                <button type="button" className="icon-btn" aria-label={`改工种 ${p.name}`} onClick={() => openProfile(p)}>
                  <Icon name="edit" />
                </button>
              </header>
              {p.note && <p className="pf-note">附加说明：{p.note}</p>}
              <div className="pfb-row">
                <span className="pfb-k">领这些卡</span>
                <div className="pfb-rules">
                  {mine.map(({ r, i }) =>
                    editing?.i === i ? (
                      <RuleEditor key={i} r={r} withTarget onCancel={() => setEditing(null)} onSave={x => (saveRule(i, x), setEditing(null))} />
                    ) : (
                      <div key={i} className="pfb-rule">
                        <span className="pfa-no">{i + 1}</span>
                        <span>{ruleText(r)}</span>
                        <RuleActs i={i} n={rules.length} onEdit={() => setEditing({ i, to: p.id })} />
                      </div>
                    ),
                  )}
                  {editing?.i === 'new' && editing.to === p.id ? (
                    <RuleEditor r={{ op: 'in', labels: [], to: p.id }} onCancel={() => setEditing(null)} onSave={x => (addRule(x), setEditing(null))} />
                  ) : (
                    <button type="button" className="pfb-add" onClick={() => setEditing({ i: 'new', to: p.id })}>
                      <Icon name="plus" />
                      加一个条件
                    </button>
                  )}
                  {!mine.length && editing?.to !== p.id && <em className="pf-dim">没有规则交给它：只能手动派发</em>}
                </div>
              </div>
              {p.agent && (
                <div className="pfb-row">
                  <span className="pfb-k">在哪跑</span>
                  <div className="pfb-hosts">
                    {hw.map(x => {
                      const off = !!p.only && !p.only.includes(x.name);
                      return (
                        <button
                          key={x.name}
                          type="button"
                          className={cx('pf-hw', off ? 't-unpicked' : `t-${x.tone}`)}
                          title={off ? `${x.name}：没选这台，点一下让它也跑「${p.name}」` : `${x.name}：${x.why}。点一下不让这台跑「${p.name}」`}
                          onClick={() => toggleOnly(p.id, x.name, all)}
                        >
                          <i />
                          <b>{x.name}</b>
                          <small>{off ? '没选' : x.why}</small>
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}
              <div className="pfb-row">
                <span className="pfb-k">现在</span>
                <div className="pf-now">
                  {l.running.map(r => (
                    <span key={r.ref} className="pf-run">
                      <i />
                      {r.host} · {r.ref} {r.title} · {r.min} 分钟
                    </span>
                  ))}
                  {[...l.starting, ...l.waiting].map(f => (
                    <button key={f.card} type="button" className={cx('pf-wait', `t-${verdict(f).tone}`)} onClick={() => goCard(f.card)}>
                      <b>#{f.card}</b> {f.title}
                      <small>{verdict(f).text}</small>
                    </button>
                  ))}
                  {!l.running.length && !l.waiting.length && !l.starting.length && <em className="pf-dim">空着</em>}
                </div>
              </div>
              <footer>
                <span className="pfb-k">近 30 天</span>
                <StatsLine s={p.stats} me={!p.agent} />
                <small className="pf-fake">假数据</small>
              </footer>
            </article>
          );
        })}
      </div>
      <p className="pfb-after">
        其余卡片（规则都没命中的）
        <Icon name="chevRight" />
        <FallbackSelect />
        {unmatched.length > 0 && (
          <span className="pf-dim">
            现在有 {unmatched.length} 张：{unmatched.map(f => `#${f.card}`).join('、')}
          </span>
        )}
      </p>
    </section>
  );
}

/* ------------------------------ C 路由表 + 试一张 ------------------------------ */

function VariantC() {
  const { profiles, rules, fallback, hosts } = useModel();
  const fc = useFc();
  const list = [...fc.values()];
  const [pick, setPick] = useState<number | 'try'>(list[0]?.card ?? 'try');
  const [tryLabels, setTryLabels] = useState<string[]>(['bug']);
  const [editing, setEditing] = useState<number | 'new' | null>(null);
  const labels = useLabelNames();
  const counts = (i: number) => list.filter(f => f.route?.rule === i).length;
  const unmatched = list.filter(f => f.kind === 'none');

  // 试一张：真卡用推算结果（排在它前面的卡已经占了位）；自己拼的标签单独算
  const cur = pick === 'try' ? null : fc.get(pick);
  const names = cur ? cur.names : tryLabels;
  const route = cur ? cur.route : routeOf(names, rules, fallback);
  const target = route && profiles.find(p => p.id === route.to);
  const hw = cur ? cur.hosts : target?.agent ? hostsFor(target, hosts, true) : [];
  const got = cur ? hw.find(x => x.picked) : pickHost(hw, hosts);
  const firstHit = route?.rule ?? -1;

  return (
    <section className="sc pfc">
      <PageHead text="左边是规则和工种；右边挑一张卡（或自己拼一组标签），一步步看它会交给谁、哪台机器、什么时候开始。" />
      <div className="pfc-grid">
        <div className="pfc-main">
          <h4>
            规则 <small>从上往下，第一条命中的生效</small>
          </h4>
          <div className="pfc-t pfc-rules">
            <div className="pfc-tr pfc-th">
              <span>#</span>
              <span>卡片满足</span>
              <span>交给</span>
              <span>现在命中</span>
              <span />
            </div>
            {rules.map((r, i) =>
              editing === i ? (
                <RuleEditor key={i} r={r} withTarget onCancel={() => setEditing(null)} onSave={x => (saveRule(i, x), setEditing(null))} />
              ) : (
                <div key={i} className={cx('pfc-tr', cur && i < firstHit && 'miss', cur && i === firstHit && 'hit')}>
                  <span className="pfa-no">{i + 1}</span>
                  <span>{ruleText(r)}</span>
                  <span>
                    <Select className="pfc-to" value={r.to} options={profiles.map(p => ({ value: p.id, label: p.name }))} aria-label="交给哪个工种" onChange={v => saveRule(i, { ...r, to: v })} />
                  </span>
                  <span className="pf-dim">{counts(i) ? `${counts(i)} 张` : '—'}</span>
                  <RuleActs i={i} n={rules.length} onEdit={() => setEditing(i)} />
                </div>
              ),
            )}
            {editing === 'new' && <RuleEditor r={{ op: 'in', labels: [], to: profiles[0]?.id ?? '' }} withTarget onCancel={() => setEditing(null)} onSave={x => (addRule(x), setEditing(null))} />}
            <div className={cx('pfc-tr fb', cur && firstHit === -1 && route && 'hit')}>
              <span className="pfa-no">·</span>
              <span>其余卡片</span>
              <span>
                <FallbackSelect />
              </span>
              <span className="pf-dim">{unmatched.length ? `${unmatched.length} 张` : '—'}</span>
              <span />
            </div>
          </div>
          <button type="button" className="btn sm pfc-add" onClick={() => setEditing('new')}>
            <Icon name="plus" />
            加一条规则
          </button>

          <h4>
            工种 <small>派发时按次带上模型和强度，不改机器上 agent 自己的配置</small>
          </h4>
          <div className="pfc-t pfc-profs">
            <div className="pfc-tr pfc-th">
              <span>工种</span>
              <span>agent · 模型 · 强度</span>
              <span>能跑的机器</span>
              <span>近 30 天</span>
              <span />
            </div>
            {profiles.map(p => (
              <div key={p.id} className={cx('pfc-tr', target?.id === p.id && 'hit')}>
                <span>
                  <PChip p={p} />
                </span>
                <span className="pfc-recipe">{p.agent ? recipe(p).replace(/^[^·]+· /, '') : '不派给机器'}</span>
                <span className="pfc-hosts">
                  {p.agent ? hosts.map(h => {
                    const c = canRun(p, h);
                    return (
                      <span key={h.name} className={cx('pfc-hd', c.ok ? 'ok' : 'bad')} title={`${h.name}：${c.why}`}>
                        {c.ok ? <Icon name="check" /> : <Icon name="close" />}
                        {h.name}
                      </span>
                    );
                  }) : <em className="pf-dim">—</em>}
                </span>
                <span className={cx('pfc-pass', p.stats.pass < 60 && 'bad')}>
                  {p.stats.pass}% · {p.stats.done} 张
                </span>
                <span>
                  <button type="button" className="icon-btn sm" aria-label={`改工种 ${p.name}`} onClick={() => openProfile(p)}>
                    <Icon name="edit" />
                  </button>
                </span>
              </div>
            ))}
          </div>
          <button type="button" className="btn sm pfc-add" onClick={() => openProfile(null)}>
            <Icon name="plus" />
            新建工种
          </button>
          <p className="pf-dim pfc-foot">「近 30 天」是一次通过率和交审张数（假数据）；点工种看全部：返工、异常、用时、费用。</p>
        </div>

        <aside className="pfc-try">
          <h4>这张卡会怎么走</h4>
          <div className="pfc-pick">
            {list.map(f => (
              <button key={f.card} type="button" className={cx('pfc-card', pick === f.card && 'on')} onClick={() => setPick(f.card)}>
                <b>#{f.card}</b>
                <span>{f.title}</span>
                <small className={`t-${verdict(f).tone}`}>{verdict(f).text}</small>
              </button>
            ))}
            <button type="button" className={cx('pfc-card try', pick === 'try' && 'on')} onClick={() => setPick('try')}>
              <Icon name="tag" />
              <span>自己拼一组标签试试</span>
            </button>
          </div>
          {pick === 'try' && (
            <div className="pfc-tl">
              {labels.map(l => (
                <button key={l} type="button" className={cx('pf-re-l', tryLabels.includes(l) && 'on')} onClick={() => setTryLabels(tryLabels.includes(l) ? tryLabels.filter(x => x !== l) : [...tryLabels, l])}>
                  {l}
                </button>
              ))}
            </div>
          )}
          <ol className="pfc-trace">
            <li className="pfc-step">
              <span className="pfc-sn">1</span>
              <div>
                <b>标签</b>
                <p>{names.length ? names.map(n => `「${n}」`).join('') : '没有标签'}</p>
              </div>
            </li>
            <li className="pfc-step">
              <span className="pfc-sn">2</span>
              <div>
                <b>规则</b>
                {rules.map((r, i) => {
                  const h = ruleHit(r, names);
                  const after = firstHit >= 0 && i > firstHit;
                  return (
                    <p key={i} className={cx('pfc-rl', after ? 'skip' : h.hit ? 'hit' : 'miss')}>
                      <span className="pfa-no">{i + 1}</span>
                      {ruleText(r)}
                      <small>{after ? '前面已经命中，不再看' : h.hit ? `命中（${h.why}）→ ${profiles.find(p => p.id === r.to)?.name}` : h.why}</small>
                    </p>
                  );
                })}
                {firstHit === -1 && <p className={cx('pfc-rl', route ? 'hit' : 'miss')}>其余卡片 <small>{route ? `→ ${target?.name}` : '不自动领'}</small></p>}
              </div>
            </li>
            {target && (
              <li className="pfc-step">
                <span className="pfc-sn">3</span>
                <div>
                  <b>工种</b>
                  <p className="pfc-pt">
                    <PChip p={target} sub={target.agent ? short(target) : undefined} />
                  </p>
                  {target.note && <p className="pf-dim">附加说明：{target.note}</p>}
                </div>
              </li>
            )}
            {target?.agent && (
              <li className="pfc-step">
                <span className="pfc-sn">4</span>
                <div>
                  <b>机器</b>
                  {hw.map(x => (
                    <p key={x.name} className={cx('pfc-hl', `t-${x.tone}`, got?.name === x.name && 'picked')}>
                      <i />
                      <b>{x.name}</b>
                      <span>{x.can ? `能跑 · ${x.why}` : x.why}</span>
                      {got?.name === x.name && <em>挑它：空位最多</em>}
                    </p>
                  ))}
                </div>
              </li>
            )}
            <li className={cx('pfc-step verdict', got ? 't-ok' : target?.agent ? 't-wait' : 't-none')}>
              <span className="pfc-sn">
                <Icon name={got ? 'play' : 'timer'} />
              </span>
              <div>
                <b>
                  {!route
                    ? '不会自动领：没有命中的规则'
                    : !target?.agent
                      ? '等你自己开会话'
                      : got
                        ? `马上在 ${got.name} 开始`
                        : cur?.kind === 'wait'
                          ? `排队：「${target.name}」第 ${cur.pos} 张`
                          : '排队'}
                </b>
                <p>
                  {!route
                    ? '可以在卡片上手动派发，或在左边加一条规则 / 设兜底。'
                    : !target?.agent
                      ? '卡片上写「等你开会话」，带「复制提示词」。'
                      : got
                        ? `${got.name} 下一次上报（最多 10 秒）就开一个会话。`
                        : soonest(hw)}
                </p>
              </div>
            </li>
          </ol>
        </aside>
      </div>
    </section>
  );
}
