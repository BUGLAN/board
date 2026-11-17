/* ==========================================================================
   「跑第一张卡」清单（default/main#397）：服务端 firstrun.rs 按白板算好（工作区数据的 checklist），这里只管怎么画
   - 每项的标题、去处按钮的文字和动作（goOf）
   - 白板页的提示什么时候出现（missing）：还没有卡被机器交审过，并且有没通过或还没确认的项；自动领取那一项只影响
     自动领取，不算（手动派发照常能跑第一张卡）
   ========================================================================== */
import type { CheckItem, Checklist, Workspace } from './types';

export const CHECK_TITLE: Record<string, string> = {
  host: '有在线、能接这块白板的机器',
  agent: '机器上有能干活的 agent',
  repo: '项目设了仓库地址',
  push: '机器能推送',
  auto: '自动领取',
};

/** 这块白板的清单；旧服务端没有时为 null */
export const checklistOf = (ws: Workspace | null, path: string): Checklist | null => ws?.checklist?.find(c => c.board === path) ?? null;

/** 还没通过、要人去处理的项（不含只影响自动领取的） */
export const missing = (c: Checklist | null): CheckItem[] => (c && !c.done ? c.items.filter(i => !i.auto && (i.state === 'fail' || i.state === 'unknown')) : []);

/** 去处按钮：文字和要做的事。kind 是动作，host 是那台机器（自检、打开机器详情时） */
export type Go =
  | { kind: 'add-host' | 'hosts' | 'project' | 'git' | 'rules' | 'accept' | 'board'; label: string }
  | { kind: 'host'; label: string; host: string; tab: 'settings' | 'agents' }
  | { kind: 'selftest'; label: string; host: string };

export function goOf(go: string | null | undefined): Go | null {
  if (!go) return null;
  const [k, host = ''] = go.split(/:(.*)/s);
  switch (k) {
    case 'add-host':
      return { kind: 'add-host', label: '添加机器' };
    case 'hosts':
      return { kind: 'hosts', label: '看机器' };
    case 'host-settings':
      return { kind: 'host', label: `打开 ${host}`, host, tab: 'settings' };
    case 'host-agents':
      return { kind: 'host', label: `${host} 的 agent`, host, tab: 'agents' };
    case 'selftest':
      return { kind: 'selftest', label: '自检', host };
    case 'project-repo':
      return { kind: 'project', label: '项目设置' };
    case 'git':
      return { kind: 'git', label: 'Git 凭据' };
    case 'rules':
      return { kind: 'rules', label: '领取规则' };
    case 'accept':
      return { kind: 'accept', label: '接单' };
    case 'board-auto':
      return { kind: 'board', label: '白板设置' };
    default:
      return null;
  }
}
