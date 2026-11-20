/* ==========================================================================
   原型（用完即弃）：卡片详情「派发」对话框的两个变体（?dl=X|Y；0 = 现在的先选机器再选 agent）
   X 先选工种：按规则会交给的那个预先选中，每个工种写现在派会怎样；机器收在下面，默认任意
   Y 工种 × 机器一张表：一行一个工种、一列一台机器，格子里写能不能跑、空几位；点行头 = 任意机器，点格子 = 指定这台
   手动派发不看接单时段（只有自动领取看）。派发只弹提示，不发请求
   ========================================================================== */
import { useEffect, useMemo, useState } from 'react';
import { cx } from '../../lib/util';
import { useData } from '../../store/data';
import { openDispatch, toast } from '../../store/ui';
import { OsGlyph } from '../../components/hosts/bits';
import { Icon } from '../../components/Icon';
import { HostWhyChip, PMark } from './bits';
import { hostsFor, pickHost, recipe, routeOf, ruleText, short, soonest, useModel, type HostWhy, type Profile } from './model';
import { useDl } from './proto';

export function ProtoDispatch({ card, on }: { card: number; on: boolean }) {
  const dl = useDl();
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const entry = useData(s => s.board?.cards[card]);
  const { profiles, rules, fallback, hosts } = useModel();
  const names = useMemo(() => {
    const labels = ws?.projects.find(p => p.key === path.split('/')[0])?.labels ?? [];
    return (entry?.labels ?? []).map(id => labels.find(l => l.id === id)?.name).filter((x): x is string => !!x);
  }, [ws, path, entry]);
  const route = routeOf(names, rules, fallback);
  const suggested = route ? profiles.find(p => p.id === route.to) : undefined;
  const [prof, setProf] = useState<string>(suggested?.id ?? profiles.find(p => p.agent)?.id ?? '');
  const [host, setHost] = useState<string>('');
  const [open, setOpen] = useState(false);
  const p = profiles.find(x => x.id === prof);
  const close = () => openDispatch(null);
  const hw = p?.agent ? hostsFor(p, hosts, false) : [];
  const got = host ? hw.find(x => x.name === host && x.ok) : pickHost(hw, hosts);

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

  const submit = () => {
    if (!p) return;
    close();
    if (!p.agent) {
      void navigator.clipboard?.writeText(`/board-skills 开始执行 ${path.split('/')[0]}项目之中的${path.split('/')[1]}白板中的#${card}任务`).catch(() => {});
      toast({ type: 'success', title: `已复制 #${card} 的提示词`, text: '（原型）贴进你自己开的会话' });
      return;
    }
    toast({
      type: 'success',
      title: `（原型）#${card} 派给「${p.name}」· ${host || '任意机器'}`,
      text: got ? `${got.name} 有空位，马上开始：${recipe(p)}` : `排队：${soonest(hw)}`,
    });
  };

  const why = route ? (route.rule >= 0 ? `规则 ${route.rule + 1}：${ruleText(rules[route.rule])}` : '兜底：其余卡片') : '没有命中的规则';

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && close()}>
      <div className={cx('dialog dp-dialog pfd', dl === 'Y' && 'wide')} role="dialog" aria-modal="true" aria-label={`派发 #${card}`}>
        <h3>派发 #{card}</h3>
        <p className="dp-sub">{entry?.title}</p>
        {dl === 'X' ? (
          <>
            <div className="dp-label pfd-label">
              工种
              <small>
                {suggested ? (
                  <>
                    按{why}，会交给「{suggested.name}」
                  </>
                ) : (
                  `${why}：自己挑一个`
                )}
              </small>
            </div>
            <div className="pfx-list" role="radiogroup" aria-label="工种">
              {profiles.map(x => (
                <ProfileRow key={x.id} p={x} on={x.id === prof} suggested={x.id === suggested?.id} onPick={() => (setProf(x.id), setHost(''))} />
              ))}
            </div>
            {p?.agent && (
              <div className="pfx-where">
                <button type="button" className="pfx-where-h" aria-expanded={open} onClick={() => setOpen(!open)}>
                  在哪台机器
                  <b>{host ? host : got ? `任意 · 现在会去 ${got.name}（空 ${got.free} 位）` : '任意 · 谁先空出来给谁'}</b>
                  <Icon name={open ? 'chevUp' : 'chevDown'} />
                </button>
                {open && (
                  <div className="pfx-hosts" role="radiogroup" aria-label="机器">
                    <button type="button" className={cx('pfx-h', host === '' && 'on')} onClick={() => setHost('')}>
                      <Icon name="dispatch" />
                      <b>任意</b>
                      <small>挑空位最多的</small>
                    </button>
                    {hw.map(x => (
                      <button key={x.name} type="button" className={cx('pfx-h', host === x.name && 'on')} disabled={!x.can} onClick={() => setHost(x.name)}>
                        <HostWhyChip x={x} />
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </>
        ) : (
          <Matrix prof={prof} host={host} suggested={suggested?.id} why={why} onPick={(pid, h) => (setProf(pid), setHost(h))} />
        )}
        {p && <Sentence p={p} host={host} hw={hw} got={got ?? null} />}
        <div className="dialog-actions">
          <button className="btn" type="button" onClick={close}>
            取消
          </button>
          <button className="btn brand" type="button" disabled={!p || (!!p.agent && !hw.some(x => x.can))} onClick={submit}>
            <Icon name={p?.agent ? 'dispatch' : 'copy'} />
            {p?.agent ? '派发' : '复制提示词'}
          </button>
        </div>
      </div>
    </div>
  );
}

/** X：一个工种一行：图标、名字、模型；右边写现在派给它会怎样 */
function ProfileRow({ p, on, suggested, onPick }: { p: Profile; on: boolean; suggested: boolean; onPick: () => void }) {
  const hosts = useModel(s => s.hosts);
  const hw = p.agent ? hostsFor(p, hosts, false) : [];
  const got = pickHost(hw, hosts);
  const can = hw.filter(x => x.can).length;
  const text = !p.agent ? '你自己开会话：复制提示词' : got ? `马上开始 · ${got.name} 空 ${got.free} 位` : can ? '会排队' : '没有机器能跑';
  const tone = !p.agent ? 'me' : got ? 'ok' : can ? 'wait' : 'bad';
  return (
    <button type="button" role="radio" aria-checked={on} className={cx('pfx-p', on && 'on')} disabled={!!p.agent && !can} onClick={onPick}>
      <PMark p={p} />
      <span className="pfx-pt">
        <b>
          {p.name}
          {suggested && <span className="pfx-tag">按规则</span>}
        </b>
        <small>{p.agent ? recipe(p) : '不派给机器'}</small>
      </span>
      <em className={`t-${tone}`}>{text}</em>
    </button>
  );
}

/** Y：工种 × 机器 */
function Matrix({ prof, host, suggested, why, onPick }: { prof: string; host: string; suggested?: string; why: string; onPick: (p: string, h: string) => void }) {
  const { profiles, hosts } = useModel();
  return (
    <>
      <div className="dp-label pfd-label">
        工种 × 机器
        <small>
          {suggested ? `按${why}，会交给「${profiles.find(p => p.id === suggested)?.name}」` : `${why}：自己挑一格`} · 点工种名 = 任意机器，点格子 = 指定这台
        </small>
      </div>
      <div className="pfy-grid" style={{ gridTemplateColumns: `minmax(150px, 1.5fr) repeat(${hosts.length}, minmax(72px, 1fr))` }}>
        <span className="pfy-c pfy-th" />
        {hosts.map(h => (
          <span key={h.name} className={cx('pfy-c pfy-th', !h.online && 'off')}>
            <OsGlyph os={h.os} small />
            <b>{h.name}</b>
            <small>{h.online ? `${h.running.length}/${h.max}` : '离线'}</small>
          </span>
        ))}
        {profiles.map(p => {
          const hw = p.agent ? hostsFor(p, hosts, false) : [];
          const rowOn = prof === p.id;
          return [
            <button key={`${p.id}-h`} type="button" className={cx('pfy-c pfy-rh', rowOn && host === '' && 'on', rowOn && 'row')} onClick={() => onPick(p.id, '')}>
              <PMark p={p} size="sm" />
              <span>
                <b>
                  {p.name}
                  {suggested === p.id && <span className="pfx-tag">按规则</span>}
                </b>
                <small>{p.agent ? short(p) : '你自己开会话'}</small>
              </span>
            </button>,
            ...(p.agent
              ? hw.map(x => (
                  <button
                    key={`${p.id}-${x.name}`}
                    type="button"
                    className={cx('pfy-c pfy-cell', `t-${x.tone}`, rowOn && host === x.name && 'on', rowOn && 'row')}
                    disabled={!x.can}
                    title={`${p.name} @ ${x.name}：${x.why}`}
                    onClick={() => onPick(p.id, x.name)}
                  >
                    {x.can ? (x.ok ? <Icon name="check" /> : <Icon name="timer" />) : <Icon name="close" />}
                    <small>{x.can ? x.why.replace('，', ' ') : x.why.split('：')[0]}</small>
                  </button>
                ))
              : [
                  <span key={`${p.id}-me`} className={cx('pfy-c pfy-me', rowOn && 'row')} style={{ gridColumn: `span ${hosts.length}` }}>
                    不派给机器
                  </span>,
                ]),
          ];
        })}
      </div>
    </>
  );
}

/** 对话框底部一句：派给谁、在哪、什么时候开始 */
function Sentence({ p, host, hw, got }: { p: Profile; host: string; hw: HostWhy[]; got: HostWhy | null }) {
  if (!p.agent) return <p className="dp-note pfd-say">不派给机器：复制提示词，贴进你自己开的会话。</p>;
  const sel = host ? hw.find(x => x.name === host) : null;
  return (
    <p className={cx('dp-note pfd-say', got ? 't-ok' : 't-wait')}>
      派给「{p.name}」· {host || '任意机器'}：
      {got ? `${got.name} 空 ${got.free} 位，马上开始。` : sel ? `${sel.name} ${sel.why}，排队等它。` : `排队，${soonest(hw)}。`}
      {p.note && <span className="pfd-note">附加说明会接在提示词后面：{p.note}</span>}
    </p>
  );
}
