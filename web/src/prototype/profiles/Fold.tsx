/* ==========================================================================
   原型（用完即弃）：卡片动态里连着的调度记录（派发、开始第 N 次、心跳中断、机器恢复连接、会话结束、board host 的交接评论……）
   收成一行摘要，点开看明细（?fold=1）。人的操作、评论、交审照常一条一条显示
   打开 #512 看：线上真实的 58 条动态，两小时里开始了 7 次
   ========================================================================== */
import { Fragment, useState, type ReactNode } from 'react';
import type { Activity } from '../../lib/types';
import { cx } from '../../lib/util';
import { Icon } from '../../components/Icon';
import { dur } from './model';

const SCHED = /^(派发|开始第|心跳中断|机器 \S+ 恢复连接|会话结束|领取了卡片（派发|停止派发|强制释放|board host（|结束了心跳|\d+ 分钟没有任何输出)/;

/** 是不是调度记录：系统和 agent 写的调度类文字；agent 领取卡片也算（派发时 board host 替它领的） */
const isSched = (a: Activity) =>
  a.actor.kind !== 'human' &&
  (SCHED.test(a.text) || (a.action === 'claim' && a.actor.kind === 'agent') || (a.action === 'move' && a.actor.kind === 'agent' && /移到「(错误|需人工校验)」/.test(a.text)));

type Item = { one: Activity } | { group: Activity[] };

function group(list: Activity[]): Item[] {
  const out: Item[] = [];
  let run: Activity[] = [];
  const flush = () => {
    if (run.length >= 3) out.push({ group: run });
    else run.forEach(a => out.push({ one: a }));
    run = [];
  };
  for (const a of list) {
    if (isSched(a)) run.push(a);
    else {
      flush();
      out.push({ one: a });
    }
  }
  flush();
  return out;
}

/** 一组调度记录的一句话：多久、开始几次、怎么结束的、涉及哪几台 */
function summary(g: Activity[]) {
  const runs = g.filter(a => a.text.startsWith('开始第')).length || g.filter(a => a.text.startsWith('派发开始')).length;
  const ends = new Map<string, number>();
  let last = '';
  for (const a of g) {
    if (!a.text.startsWith('派发结束：')) continue;
    const r = a.text.slice(5).split(/[（，]/)[0];
    ends.set(r, (ends.get(r) ?? 0) + 1);
    last = r;
  }
  const hosts = new Set<string>();
  for (const a of g) {
    const m = a.text.match(/^派发到 (\S+) ·|^派发开始：(\S+) 上的|^机器 (\S+) 恢复连接/);
    const h = m?.[1] ?? m?.[2] ?? m?.[3];
    if (h) hosts.add(h);
  }
  const revived = g.filter(a => a.text.includes('恢复连接')).length;
  const min = Math.max(1, Math.round((g[g.length - 1].at - g[0].at) / 60_000));
  // 最后一次怎么结束的单独写在箭头后面；它只出现一次时不再在前面重复
  const tail = last && ends.get(last) === 1 ? last : '';
  const rest = [...ends].filter(([k]) => k !== tail).map(([k, n]) => (n > 1 ? `${k} ${n} 次` : k)).join('、');
  const moved = g.map(a => a.text.match(/移到「(.+?)」/)?.[1]).filter(Boolean).pop();
  return {
    head: `调度 · ${dur(min)}：开始 ${runs} 次${rest ? `，${rest}` : ''}${tail ? ` → 最后${tail}` : ''}`,
    sub: [hosts.size ? `涉及 ${[...hosts].join('、')}` : '', revived ? `掉线后接回 ${revived} 次` : '', moved ? `卡片进了「${moved}」` : '', `收起了 ${g.length} 条`].filter(Boolean).join(' · '),
    bad: last && !/交审|拆解/.test(last),
  };
}

export function Folded({ list, render }: { list: Activity[]; render: (a: Activity, idx: number) => ReactNode }) {
  const items = group(list);
  let idx = 0;
  return (
    <>
      {items.map(it => {
        if ('one' in it) return <Fragment key={it.one.id}>{render(it.one, idx++)}</Fragment>;
        const start = idx;
        idx += it.group.length;
        return <FoldRow key={`g${it.group[0].id}`} g={it.group} render={a => render(a, start + it.group.indexOf(a))} />;
      })}
    </>
  );
}

function FoldRow({ g, render }: { g: Activity[]; render: (a: Activity) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const s = summary(g);
  return (
    <div className={cx('pf-fold', open && 'open', s.bad && 'bad')}>
      <button type="button" className="pf-fold-h" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="tl-icon k-dispatch">
          <Icon name="dispatch" />
        </span>
        <span className="pf-fold-t">
          <b>{s.head}</b>
          <small>{s.sub}</small>
        </span>
        <Icon name={open ? 'chevUp' : 'chevDown'} />
      </button>
      {open && <div className="pf-fold-body">{g.map(a => <Fragment key={a.id}>{render(a)}</Fragment>)}</div>}
    </div>
  );
}
