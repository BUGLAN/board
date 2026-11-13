/* ==========================================================================
   共用配置的「agent 模板」和机器详情的「复制设置到…」（PRD §10.12，卡片 #188）：界面上用到的纯函数
   - 模板的编辑表单：命令、参数一行一个；强度用空格或逗号隔开；Windows 启动环境按勾选的顺序。校验和服务端一致
   - 一个模板在各台机器上的情况：找到 / 没找到 / 本机配置覆盖 / 还没上报 / 离线
   ========================================================================== */
import type { AgentTemplate, HostInfo } from './types';

/** 复制设置能选的项（和服务端 COPY_ITEMS 一致） */
export const COPY_ITEMS: [string, string][] = [
  ['limits', '最多几个会话和一个会话最多做几张'],
  ['cron', '定时领取'],
  ['rules', '领取规则和兜底'],
  ['agent', '默认 agent'],
  ['run', '每个 agent 的启动环境、模型、强度'],
];

/** Windows 上能选的启动环境（和服务端 WIN_ENVS 一致） */
export const WIN_ENVS: [string, string][] = [
  ['pwsh', 'PowerShell 7'],
  ['powershell', 'Windows PowerShell'],
  ['git-bash', 'Git Bash'],
  ['wsl:*', 'WSL'],
  ['cmd', 'cmd'],
  ['direct', '直接启动'],
];

/** 表单：多值的字段是文本（一行一个、或空格隔开），保存时再拆 */
export interface TemplateForm {
  id: string;
  name: string;
  cmd: string;
  model: string;
  modelArgs: string;
  effortArgs: string;
  efforts: string;
  win: string[];
}

export const blankForm = (): TemplateForm => ({ id: '', name: '', cmd: '', model: '', modelArgs: '', effortArgs: '', efforts: '', win: [] });

const lines = (t: string) => t.split('\n').map(s => s.trim()).filter(Boolean);
const words = (t: string) => t.split(/[\s,，、]+/).map(s => s.trim()).filter(Boolean);

export function toForm(t: AgentTemplate): TemplateForm {
  return {
    id: t.id,
    name: t.name,
    cmd: t.cmd.join('\n'),
    model: t.model,
    modelArgs: t.modelArgs.join('\n'),
    effortArgs: t.effortArgs.join('\n'),
    efforts: t.efforts.join(' '),
    win: [...t.win],
  };
}

export function fromForm(f: TemplateForm): AgentTemplate {
  return {
    id: f.id.trim(),
    name: f.name.trim(),
    cmd: lines(f.cmd),
    model: f.model.trim(),
    modelArgs: lines(f.modelArgs),
    effortArgs: lines(f.effortArgs),
    efforts: words(f.efforts),
    win: WIN_ENVS.map(([k]) => k).filter(k => f.win.includes(k)),
  };
}

/** 校验（和服务端一致）：不对时返回原因 */
export function templateError(t: AgentTemplate, existing: string[], editing: boolean): string | null {
  if (!/^[a-z][a-z0-9.-]{0,15}$/.test(t.id)) return '标识只能用小写字母、数字、.、-，以字母开头，最长 16 个字符';
  if (!editing && existing.includes(t.id)) return `已经有「${t.id}」了：点它修改`;
  if ([...t.name].length > 32) return '名称最长 32 个字符';
  if (!t.cmd.length) return '命令模板不能为空：第一行是命令，再写一行 {prompt}';
  if (t.cmd[0].includes('{prompt}')) return '命令模板的第一行要是命令本身，{prompt} 写在后面';
  const n = t.cmd.filter(a => a.includes('{prompt}')).length;
  if (n !== 1) return `命令模板里要正好有一个 {prompt}（现在有 ${n} 个）`;
  if (t.model && !/^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,63}$/.test(t.model)) return '默认模型只能用字母、数字和 . _ : / @ + -，最长 64 个字符';
  if (t.modelArgs.length && !t.modelArgs.some(a => a.includes('{model}'))) return '传模型的参数里要有 {model}，例如 --model {model}';
  if (t.effortArgs.length && !t.effortArgs.some(a => a.includes('{effort}'))) return '传思考强度的参数里要有 {effort}，例如 --effort {effort}';
  const bad = t.efforts.find(e => !/^[A-Za-z0-9_-]{1,16}$/.test(e));
  if (bad) return `思考强度「${bad}」不对：只能用字母、数字、-、_`;
  return null;
}

/** 命令模板的一行展示：参数里有空格的加引号 */
export const cmdLine = (cmd: string[]) => cmd.map(a => (/\s/.test(a) ? `"${a}"` : a)).join(' ');

export type TemplateCell = { host: HostInfo; state: 'found' | 'missing' | 'local' | 'pending' | 'offline'; version?: string };

/** 一个模板在每台机器上的情况 */
export function templateRows(hosts: HostInfo[], t: AgentTemplate): TemplateCell[] {
  return hosts.map(h => {
    // 离线的机器还没取到这份模板：不按它上一次的上报算
    if (!h.online) return { host: h, state: 'offline' };
    const a = h.agents.find(x => x.id === t.id);
    if (a) return { host: h, state: a.local_override ? 'local' : a.found ? 'found' : 'missing', version: a.version || undefined };
    return { host: h, state: 'pending' };
  });
}

export const CELL_WORDS: Record<TemplateCell['state'], string> = {
  found: '找到了',
  missing: '没找到命令',
  local: '本机配置覆盖',
  pending: '还没上报',
  offline: '离线',
};
