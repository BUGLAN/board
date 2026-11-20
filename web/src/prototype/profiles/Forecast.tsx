/* ==========================================================================
   原型（用完即弃）：卡片会交给谁
   - 卡片详情里一段「接下来」：交给哪个工种（哪条规则）、马上在哪台开始 / 排第几、等什么；「为什么」展开每台机器的原因
   - 白板卡片上一行去向（?chip=1）：「→ 修复 · 马上开始」「→ 批量 · 排队」「→ 我」「不自动领」
   只画可领取、没人持有、没派发的卡；推算见 model.ts
   ========================================================================== */
import { useMemo, useState } from 'react';
import { cardDispatch } from '../../lib/hosts';
import type { BoardSnapshot, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { useData } from '../../store/data';
import { openDispatch, toast } from '../../store/ui';
import { Icon } from '../../components/Icon';
import { HostWhyChip, PChip, PMark } from './bits';
import { claimableCards, forecast, recipe, ruleText, useModel, type Fc } from './model';
import { useChip } from './proto';

/** 一块白板的推算，按 (工作区, 白板, 规则和机器) 缓存：白板上每张卡片都要问一次 */
let cache: { ws: Workspace | null; b: BoardSnapshot | null; m: unknown; out: Map<number, Fc> } | null = null;
function useBoardFc() {
  const ws = useData(s => s.ws);
  const b = useData(s => s.board);
  const m = useModel();
  if (!cache || cache.ws !== ws || cache.b !== b || cache.m !== m) cache = { ws, b, m, out: forecast(claimableCards(ws, b), m) };
  return cache.out;
}

export function CardForecast({ card }: { card: number }) {
  const fc = useBoardFc();
  const f = fc.get(card);
  const dispatched = useData(s => cardDispatch(s.ws, s.path, card));
  const { profiles, rules } = useModel();
  const path = useData(s => s.path);
  const [open, setOpen] = useState(false);
  if (!f || dispatched) return null;
  const p = f.route ? profiles.find(x => x.id === f.route!.to) : undefined;
  const why = f.route ? (f.route.rule >= 0 ? `规则 ${f.route.rule + 1}：${ruleText(rules[f.route.rule])}` : '兜底：其余卡片') : '';
  return (
    <section className={cx('d-section pf-fc', `k-${f.kind}`)} data-part="forecast">
      <div className="d-label">
        接下来<small className="pf-fake">原型</small>
      </div>
      {f.kind === 'none' ? (
        <div className="pf-fc-row">
          <span className="pf-mark">–</span>
          <div className="pf-fc-t">
            <b>不会自动领</b>
            <small>没有命中的规则，其余卡片设的是「不自动领」</small>
          </div>
          <button type="button" className="btn sm" onClick={() => openDispatch(card)}>
            <Icon name="dispatch" />
            派发…
          </button>
        </div>
      ) : (
        p && (
          <>
            <div className="pf-fc-row">
              <PMark p={p} />
              <div className="pf-fc-t">
                <b>
                  交给「{p.name}」<small>{p.agent ? recipe(p) : '不派给机器'}</small>
                </b>
                <small>{why}</small>
              </div>
            </div>
            <div className={cx('pf-fc-say', `t-${f.kind}`)}>
              <Icon name={f.kind === 'start' ? 'play' : f.kind === 'me' ? 'user' : 'timer'} />
              {f.kind === 'start' && (
                <span>
                  马上在 <b>{f.host}</b> 开始（{f.hosts.find(x => x.picked)?.why}，能跑的机器里空位最多）
                </span>
              )}
              {f.kind === 'wait' && (
                <span>
                  排队：「{p.name}」第 <b>{f.pos}</b> 张 · {f.soon}
                </span>
              )}
              {f.kind === 'me' && <span>等你自己开会话</span>}
              <span className="hp-sp" />
              {f.kind === 'me' ? (
                <button
                  type="button"
                  className="btn sm"
                  onClick={() => {
                    void navigator.clipboard?.writeText(`/board-skills 开始执行 ${path.split('/')[0]}项目之中的${path.split('/')[1]}白板中的#${card}任务`).catch(() => {});
                    toast({ type: 'success', title: `已复制 #${card} 的提示词` });
                  }}
                >
                  <Icon name="copy" />
                  复制提示词
                </button>
              ) : (
                <button type="button" className="pf-fc-more" aria-expanded={open} onClick={() => setOpen(!open)}>
                  为什么
                  <Icon name={open ? 'chevUp' : 'chevDown'} />
                </button>
              )}
            </div>
            {open && f.kind !== 'me' && (
              <div className="pf-fc-hosts">
                {f.hosts.map(x => (
                  <HostWhyChip key={x.name} x={x} />
                ))}
                <small className="pf-dim">挑机器：能跑这个工种、在接单时段里、有空位的里面挑空位最多的，一样多挑最久没接的。</small>
              </div>
            )}
            <p className="pf-fc-alt">
              不想这样？
              <button type="button" onClick={() => openDispatch(card)}>
                换个工种派发…
              </button>
            </p>
          </>
        )
      )}
    </section>
  );
}

/** 白板卡片上的去向（?chip=1） */
export function CardRouteChip({ card }: { card: number }) {
  const on = useChip();
  const fc = useBoardFc();
  const profiles = useModel(s => s.profiles);
  const f = on ? fc.get(card) : undefined;
  const p = useMemo(() => (f?.route ? profiles.find(x => x.id === f.route!.to) : undefined), [f, profiles]);
  if (!f) return null;
  if (f.kind === 'none') return <div className="pf-cchip k-none">不自动领</div>;
  if (!p) return null;
  return (
    <div className={cx('pf-cchip', `k-${f.kind}`)}>
      <Icon name="chevRight" />
      <PChip p={p} />
      <small>{f.kind === 'start' ? `马上 · ${f.host}` : f.kind === 'wait' ? `排队第 ${f.pos}` : '等你开会话'}</small>
    </div>
  );
}
