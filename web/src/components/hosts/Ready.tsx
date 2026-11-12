/* 机器就绪：机器详情顶部的就绪清单、机器 Tab 的对比视图、磁贴上的「还差 N 项」；检查本身在 lib/ready.ts */
import { Fragment, useEffect, useState } from 'react';
import { api } from '../../lib/api';
import { hostLoad } from '../../lib/hosts';
import { compareHosts, type ReadyCheck, type ReadyGo } from '../../lib/ready';
import type { HostInfo, Workspace } from '../../lib/types';
import { cx } from '../../lib/util';
import { openHost } from '../../store/ui';
import { Icon } from '../Icon';
import { AgentIcon, Dot } from './bits';

/** 服务端版本（「board 版本不落后于服务端」用）：几个地方共用一次请求，10 分钟后再取；取不到时为 null，这一项不列 */
let info: { at: number; p: Promise<string | null> } | null = null;
const INFO_TTL = 10 * 60 * 1000;
export function useServerVersion() {
  const [v, setV] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    if (!info || Date.now() - info.at > INFO_TTL) {
      const p = api.info().then(
        r => r.version || null,
        () => {
          info = null;
          return null;
        },
      );
      info = { at: Date.now(), p };
    }
    void info.p.then(x => alive && setV(x));
    return () => {
      alive = false;
    };
  }, []);
  return v;
}

/** 磁贴上的小标记：没有全部就绪时写「还差 N 项」，鼠标放上去列出是哪几项 */
export function ReadyBadge({ checks }: { checks: ReadyCheck[] }) {
  const miss = checks.filter(c => !c.ok);
  if (!miss.length) return null;
  return (
    <span className="hp-ready-badge" title={`还没就绪：${miss.map(c => c.label).join('、')}（点开看详情）`}>
      还差 {miss.length} 项
    </span>
  );
}

/** 机器详情顶部的「就绪」：一行「N/M 项通过」，展开是清单。有没通过的项时默认展开，全部通过时默认收起；点标题切换 */
export function ReadySection({ checks, go, i }: { checks: ReadyCheck[]; go: (to: ReadyGo) => void; i: number }) {
  const pass = checks.filter(c => c.ok).length;
  const all = pass === checks.length;
  const [open, setOpen] = useState<boolean | null>(null);
  const shown = open ?? !all;
  return (
    <section className="d-section hp-ready" style={{ ['--i' as string]: i }}>
      <button type="button" className="d-label hp-fold-btn" aria-expanded={shown} onClick={() => setOpen(!shown)}>
        <Icon name="chevDown" />
        就绪
        <span className={cx('hp-ready-n', all ? 'ok' : 'bad')}>
          {pass}/{checks.length} 项通过
        </span>
      </button>
      {shown && (
        <ul className="hp-ready-list hp-fold-body">
          {checks.map(c => (
            <li key={c.key} className={c.ok ? 'ok' : 'bad'}>
              <Icon name={c.ok ? 'checkCircle' : 'alert'} />
              <div>
                <b>{c.label}</b>
                {!c.ok && (
                  <span>
                    {c.why}
                    {c.hint && <small>{c.hint}</small>}
                  </span>
                )}
              </div>
              {!c.ok && c.go && (
                <button type="button" className="btn sm" onClick={() => go(c.go!)}>
                  {c.act}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** 对比视图：每台机器一行；列是就绪的每一项、board 版本、每个 agent 的版本 / 模型 / 启动环境。和其他机器不一致的格子标出来 */
export function Compare({ hosts, ws, now, server }: { hosts: HostInfo[]; ws: Workspace | null; now: number; server: string | null }) {
  const groups = compareHosts(hosts, now, server);
  const cols = groups.flatMap(g => g.cols);
  const diffs = cols.reduce((n, c) => n + c.cells.filter(x => x.diff).length, 0);
  return (
    <div className="hp-cmp-box enter" style={{ ['--i' as string]: 0 }}>
      <div className="hp-cmp-scroll">
        <table className="hp-cmp">
          <thead>
            <tr>
              <th rowSpan={2} className="hp-cmp-host" scope="col">
                机器
              </th>
              {groups.map(g => (
                <th key={g.key} colSpan={g.cols.length} className="hp-cmp-g" scope="colgroup">
                  <span>
                    {g.agent && <AgentIcon id={g.agent} />}
                    {g.title}
                  </span>
                </th>
              ))}
            </tr>
            <tr>
              {groups.map(g => (
                <Fragment key={g.key}>
                  {g.cols.map((c, j) => (
                    <th key={c.key} scope="col" className={cx(j === 0 && 'first')}>
                      {c.title}
                    </th>
                  ))}
                </Fragment>
              ))}
            </tr>
          </thead>
          <tbody>
            {hosts.map((h, r) => (
              <tr key={h.id} className={cx(!h.online && 'off')}>
                <th scope="row" className="hp-cmp-host">
                  <button type="button" onClick={() => openHost(h.name)} title={`打开 ${h.name} 的详情`}>
                    <Dot h={h} running={hostLoad(ws, h).running.length} />
                    {h.name}
                  </button>
                </th>
                {groups.map(g => (
                  <Fragment key={g.key}>
                    {g.cols.map((c, j) => {
                      const x = c.cells[r];
                      // 就绪的每一项画成 ✓ / ✗（读屏读「通过 / 没通过」），其他格子写值
                      const mark = x.ok != null && c.key !== 'ready:n';
                      return (
                        <td
                          key={c.key}
                          className={cx(j === 0 && 'first', x.diff && 'diff', x.ok === true && 'ok', x.ok === false && 'bad', x.text === '—' && 'none')}
                          title={x.diff ? `和其他机器不一致${x.tip ? `：${x.tip}` : ''}` : x.tip}
                        >
                          {mark ? (
                            <>
                              <Icon name={x.ok ? 'check' : 'close'} />
                              <span className="sr-only">{x.text}</span>
                            </>
                          ) : (
                            x.text
                          )}
                        </td>
                      );
                    })}
                  </Fragment>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="hp-cmp-legend">
        <span>
          <i className="hp-cmp-key diff" />
          和其他机器不一致{diffs ? `（${diffs} 格）` : '：没有'}
        </span>
        <span>
          <Icon name="close" />
          就绪项没通过，鼠标放上去看原因
        </span>
        <span>启动环境只和同一种系统的机器比</span>
      </p>
    </div>
  );
}
