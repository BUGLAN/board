/* ==========================================================================
   Agent tab 的侧栏目录（PRD §10.16）：三组，组名是小标题，下面是节；一节一页，点哪一节主区只画哪一节
   单独成文件：侧栏常驻，Agent 页本身按需加载
   ========================================================================== */
import { create } from 'zustand';
import { cx } from '../../lib/util';
import { Icon, type IconName } from '../Icon';

export type SecId = 'providers' | 'mcp' | 'skills' | 'git' | 'rules' | 'crons' | 'limits' | 'access' | 'tokens';

/** 三组和顺序：共用配置 → 自动领取 → 接入（接入不常用，放最后）。模型供应商的图标不用星形，免得和 tab 图标重复 */
export const GROUPS: { id: string; title: string; secs: { id: SecId; title: string; icon: IconName }[] }[] = [
  {
    id: 'shared',
    title: '共用配置',
    secs: [
      { id: 'providers', title: '模型供应商', icon: 'dispatch' },
      { id: 'mcp', title: 'MCP 服务器', icon: 'bolt' },
      { id: 'skills', title: 'skills', icon: 'file' },
      { id: 'git', title: 'Git 凭据', icon: 'lock' },
    ],
  },
  {
    id: 'auto',
    title: '自动领取',
    secs: [
      { id: 'rules', title: '领取规则', icon: 'claim' },
      { id: 'crons', title: '定时建卡', icon: 'renew' },
      { id: 'limits', title: '跑多久停', icon: 'timer' },
    ],
  },
  {
    id: 'access',
    title: '接入',
    secs: [
      { id: 'access', title: '接入 agent', icon: 'terminal' },
      { id: 'tokens', title: '访问令牌', icon: 'key' },
    ],
  },
];

export const groupOf = (sec: SecId) => GROUPS.find(g => g.secs.some(s => s.id === sec))!;
export const secOf = (sec: SecId) => GROUPS.flatMap(g => g.secs).find(s => s.id === sec)!;

/** 当前画的是哪一节：进 tab 默认落在第一节（模型供应商），这次打开页面里切回来还在上次那一节 */
export const useAgentSec = create<{ sec: SecId }>(() => ({ sec: GROUPS[0].secs[0].id }));

/** 换一节：主区回到顶部 */
export function goSec(sec: SecId) {
  useAgentSec.setState({ sec });
  const page = document.querySelector<HTMLElement>('.page-agent');
  if (page) page.scrollTop = 0;
}

export function AgentNav() {
  const sec = useAgentSec(s => s.sec);
  return (
    <nav className="settings-nav ag-nav" aria-label="Agent 的各节">
      {GROUPS.map(g => (
        <div key={g.id} className="ag-nav-g" role="group" aria-label={g.title}>
          <div className="ag-nav-h">{g.title}</div>
          {g.secs.map(s => (
            <button key={s.id} type="button" className={cx('sn-item', sec === s.id && 'on')} aria-current={sec === s.id ? 'page' : undefined} onClick={() => goSec(s.id)}>
              <Icon name={s.icon} />
              <span>{s.title}</span>
            </button>
          ))}
        </div>
      ))}
    </nav>
  );
}
