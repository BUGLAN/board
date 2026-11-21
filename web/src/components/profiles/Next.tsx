/* ==========================================================================
   卡片会交给谁（PRD §9.1「分工」）：只画可领取、没人持有、没派发的卡，推算见 lib/profiles.ts（和自动领取同一套顺序）
   - 卡片详情里一段「接下来」：交给哪个角色（按哪条规则）、马上在哪台开始还是在这个角色里排第几、在等什么；
     「为什么」展开每台机器能不能接、为什么。规则都没命中时写「不会自动领」，带「派发…」
   - 白板卡片上一行去向：「→ 修复 · 马上 · win」「→ 批量 · 排队第 1」「不自动领」
   还没有角色（没配过分工）时都不画
   ========================================================================== */
import { useState } from 'react';
import { cardDispatch } from '../../lib/hosts';
import { forecast, profilesOf, recipe, ruleWhy, type Fc } from '../../lib/profiles';
import type { BoardSnapshot, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { useData } from '../../store/data';
import { openDispatch } from '../../store/ui';
import { useNow } from '../bits';
import { Icon } from '../Icon';
import { PMark, RuleView, agentName } from './bits';

/** 一块白板的推算，按（工作区, 白板, 分钟）缓存：白板上每张卡片都要问一次 */
let cache: { ws: Workspace | null; b: BoardSnapshot | null; t: number; out: Map<number, Fc> } | null = null;
function useBoardFc(): Map<number, Fc> {
  const ws = useData(s => s.ws);
  const b = useData(s => s.board);
  const now = useNow(60_000);
  const t = Math.floor(now / 60_000);
  if (!cache || cache.ws !== ws || cache.b !== b || cache.t !== t) cache = { ws, b, t, out: forecast(ws, b, now) };
  return cache.out;
}

export function CardNext({ card }: { card: number }) {
  const fc = useBoardFc();
  const ws = useData(s => s.ws);
  const dispatched = useData(s => cardDispatch(s.ws, s.path, card));
  const [open, setOpen] = useState(false);
  const f = fc.get(card);
  const profiles = profilesOf(ws);
  if (!f || dispatched || !profiles.length) return null;
  const p = f.route ? profiles.find(x => x.id === f.route!.profile) : undefined;
  const rule = f.route ? ws?.sched?.rules[f.route.rule] : undefined;
  return (
    <section className={cx('d-section pf-fc', `k-${f.kind}`)} data-part="next">
      <div className="d-label">接下来</div>
      {f.kind === 'none' || !p ? (
        <div className="pf-fc-row">
          <span className="pf-mark">–</span>
          <div className="pf-fc-t">
            <b>不会自动领</b>
            <small>没有命中的规则，可以手动派发</small>
          </div>
          <button type="button" className="btn sm" onClick={() => openDispatch(card)}>
            <Icon name="dispatch" />
            派发…
          </button>
        </div>
      ) : (
        <>
          <div className="pf-fc-row">
            <PMark p={p} />
            <div className="pf-fc-t">
              <b>
                交给「{p.name}」<small>{recipe(p, agentName)}</small>
              </b>
              {rule && (
                <small className="pf-fc-rule">
                  规则 {f.route!.rule + 1}：<RuleView r={rule} />
                  <span className="pf-dim">（{ruleWhy(rule, f.names).why}）</span>
                </small>
              )}
            </div>
          </div>
          <div className={cx('pf-fc-say', `t-${f.kind}`)}>
            <Icon name={f.kind === 'start' ? 'play' : 'timer'} />
            {f.kind === 'start' && (
              <span>
                马上在 <b>{f.host}</b> 开始（{f.hosts.find(x => x.host.name === f.host)?.why}，能跑的机器里空位最多）
              </span>
            )}
            {f.kind === 'wait' && (
              <span>
                排队：「{p.name}」第 <b>{f.pos}</b> 张 · {f.soon}
              </span>
            )}
            {f.kind === 'off' && <span>这块白板不参与自动领取：可以手动派发</span>}
            <span className="hp-sp" />
            <button type="button" className="pf-fc-more" aria-expanded={open} onClick={() => setOpen(!open)}>
              为什么
              <Icon name={open ? 'chevUp' : 'chevDown'} />
            </button>
          </div>
          {open && (
            <div className="pf-fc-hosts">
              {f.hosts.map(x => (
                <span key={x.host.id} className={cx('pf-hw', `t-${x.tone}`)} title={`${x.host.name}：${x.why}`}>
                  <i />
                  <b>{x.host.name}</b>
                  <small>{x.why}</small>
                </span>
              ))}
              <small className="pf-dim">挑机器：能跑这个角色、在接单时段里、有空位的里面挑空位最多的，一样多挑最久没接的。</small>
            </div>
          )}
          <p className="pf-fc-alt">
            不想这样？
            <button type="button" onClick={() => openDispatch(card)}>
              换个角色派发…
            </button>
          </p>
        </>
      )}
    </section>
  );
}

/** 白板卡片上的去向 */
export function CardRoute({ card }: { card: number }) {
  const fc = useBoardFc();
  const profiles = useData(s => s.ws?.sched?.profiles);
  const f = fc.get(card);
  if (!f || !profiles?.length) return null;
  if (f.kind === 'none') return <div className="pf-cchip k-none">不自动领</div>;
  const p = f.route ? profiles.find(x => x.id === f.route!.profile) : undefined;
  if (!p) return null;
  return (
    <div className={cx('pf-cchip', `k-${f.kind}`)}>
      <Icon name="chevRight" />
      <span className="pf-chip">
        <PMark p={p} />
        <b>{p.name}</b>
      </span>
      <small>{f.kind === 'start' ? `马上 · ${f.host}` : f.kind === 'wait' ? `排队第 ${f.pos}` : '不自动领取'}</small>
    </div>
  );
}
