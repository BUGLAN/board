/* ==========================================================================
   「跑第一张卡」清单（default/main#397）：服务端按白板算好（firstrun.rs），这里只照着画
   - FirstRunList：一项一行——状态图标、标题、一句原因、去处按钮。机器 Tab 没有机器时的空状态、第一张卡跑完之前的
     总览顶部都用它（当前白板的那一份）
   - FirstRunHint：白板页页头下面一条提示，这块白板还没有卡被机器交审过、清单里有没通过或还没确认的项时出现；
     默认收着只写一行（差几项、是哪几项），点开看整份清单，展开收起按白板记在本机
   - 去处：添加机器、机器详情（设置 / agent）、自检（等服务端，按钮转圈）、项目设置、Agent tab 的 Git 凭据和领取规则、
     机器 Tab 的接单、白板设置
   ========================================================================== */
import { useState } from 'react';
import { api } from '../lib/api';
import { CHECK_TITLE, checklistOf, goOf, missing, type Go } from '../lib/firstRun';
import type { CheckItem, Checklist } from '../lib/types';
import { cx, pref } from '../lib/util';
import { optimistic, useData } from '../store/data';
import { openBoardSettings, openHost, openProjectSettings, setHostsView, setTab, toast, toastError } from '../store/ui';
import { goSec } from './agent/AgentNav';
import { Icon } from './Icon';

const STATE_ICON: Record<CheckItem['state'], string> = { ok: 'checkCircle', fail: 'alert', unknown: 'info', wait: 'timer' };

function run(g: Go, path: string) {
  const project = path.split('/')[0];
  switch (g.kind) {
    case 'add-host':
      return openHost('new');
    case 'hosts':
      setTab('hosts');
      return setHostsView('racks');
    case 'host':
      return openHost(g.host, g.tab);
    case 'project':
      return openProjectSettings(project);
    case 'git':
    case 'rules':
      setTab('agent');
      return goSec(g.kind);
    case 'accept':
      setTab('hosts');
      return setHostsView('sched');
    case 'board':
      return openBoardSettings(path);
  }
}

function GoButton({ go, path }: { go: string | null | undefined; path: string }) {
  const [busy, setBusy] = useState(false);
  const g = goOf(go);
  if (!g) return null;
  const click = async () => {
    if (g.kind !== 'selftest') return run(g, path);
    setBusy(true);
    try {
      await optimistic(null, () => api.selfTest(g.host));
      toast({ type: 'success', title: `已通知 ${g.host} 自检`, text: '检查 agent 能不能回话、每个项目的目录能不能推送，做完清单跟着更新' });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <button type="button" className={cx('btn sm', busy && 'is-busy')} disabled={busy} onClick={() => void click()}>
      {g.label}
    </button>
  );
}

export function FirstRunList({ c, path }: { c: Checklist; path: string }) {
  return (
    <ol className="fr-list">
      {c.items.map(i => (
        <li key={i.key} className={cx('fr-row', `s-${i.state}`)}>
          <Icon name={STATE_ICON[i.state]} />
          <div>
            <b>
              {CHECK_TITLE[i.key] ?? i.key}
              {i.auto && <em>只影响自动领取</em>}
            </b>
            <span>{i.why}</span>
          </div>
          {i.state !== 'ok' && <GoButton go={i.go} path={path} />}
        </li>
      ))}
    </ol>
  );
}

/** 机器 Tab：当前白板的清单（没有机器时、这块白板第一张卡跑完之前） */
export function FirstRunPanel({ empty }: { empty?: boolean }) {
  const ws = useData(s => s.ws);
  const path = useData(s => s.path);
  const c = checklistOf(ws, path);
  if (!c || (!empty && c.done)) return null;
  const left = c.items.filter(i => !i.auto && i.state !== 'ok').length;
  return (
    <section className="fr-panel" aria-label="跑第一张卡还差什么">
      <div className="fr-head">
        <b>{path} 跑第一张卡</b>
        <span>{left ? `还差 ${left} 项` : '都齐了：可以派一张卡试试'}</span>
      </div>
      <FirstRunList c={c} path={path} />
    </section>
  );
}

/** 白板页页头下面：还没有卡被机器交审过、清单里有没通过或还没确认的项时一条提示，点开看清单 */
export function FirstRunHint({ path }: { path: string }) {
  const ws = useData(s => s.ws);
  const c = checklistOf(ws, path);
  const [open, setOpen] = useState(() => pref.get<boolean>(`firstrun.open.${path}`, false));
  const left = missing(c);
  if (!c || !left.length) return null;
  const toggle = () => {
    pref.set(`firstrun.open.${path}`, !open || null);
    setOpen(!open);
  };
  return (
    <div className={cx('fr-hint', open && 'open')} role="status">
      <button type="button" className="fr-hint-bar" aria-expanded={open} onClick={toggle}>
        <Icon name="info" />
        <span>
          跑第一张卡还差 {left.length} 项：{left.map(i => CHECK_TITLE[i.key] ?? i.key).join('、')}
        </span>
        <em>{open ? '收起' : '看清单'}</em>
        <Icon name={open ? 'chevUp' : 'chevDown'} />
      </button>
      {open && <FirstRunList c={c} path={path} />}
    </div>
  );
}
