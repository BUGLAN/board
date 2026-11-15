/* 机器设置的保存（机器详情、调度共用）：每一项各存各的（store 的 saveSetting），步进器 */
import { useEffect, useRef } from 'react';
import { api } from '../../lib/api';
import type { HostAgent, HostInfo } from '../../lib/types';
import { hostKey, saveSetting } from '../../store/data';
import { useSync } from '../bits';

export type HostPatch = Parameters<typeof api.updateHost>[1];
export type Save = (p: HostPatch) => void;

/** 改的是哪个设置项：保存状态、连点合并都按它分。领取规则和兜底算同一项，启动环境按 agent 分，
    一个 agent 的模型（怎么配、供应商、模型、强度）算一项，按发出的先后一个接一个存 */
const fieldOf = (p: HostPatch) =>
  p.env
    ? `env:${p.env.agent}`
    : p.resume
      ? `resume:${p.resume}`
      : p.run
        ? `run:${p.run.agent}`
        : 'rules' in p || 'fallback' in p
          ? 'rules'
          : (Object.keys(p)[0] ?? 'host');

/** 改一个 agent 的启动环境、模型怎么配（当帧画出来） */
const agentPatch = (a: HostAgent, p: HostPatch): HostAgent => {
  if (p.env?.agent === a.id) return { ...a, env: p.env.env };
  if (p.resume === a.id) return { ...a, tripped: false };
  if (p.run?.agent === a.id) {
    const r = p.run;
    if (r.mode !== 'own') return { ...a, run_mode: r.mode, run_provider: null, run_model: null, run_effort: null };
    const provider = r.provider ?? a.run_provider ?? null;
    const same = provider === a.run_provider;
    return {
      ...a,
      run_mode: 'own',
      run_provider: provider,
      run_model: r.model !== undefined ? r.model || null : same ? a.run_model : null,
      run_effort: r.effort !== undefined ? r.effort || null : same ? a.run_effort : null,
    };
  }
  return a;
};

/** 改了会写进 agent 配置的（每个 agent 的模型、启动环境、skills / MCP 的开关）：当帧画成「正在写入」，服务端下发同步请求后以它为准 */
const touchesConfig = (p: HostPatch) => !!(p.run || p.env || p.syncSkills !== undefined || p.syncMcp !== undefined);
const syncing = (x: HostInfo): HostInfo['sync'] => (x.sync?.able ? { ...x.sync, ask: x.sync.ask ?? { id: 0, key: '', by: '', at: Date.now() } } : x.sync);

/** 步进器连点：停手这么久才发请求，只发最后的值 */
const STEPPED = new Set(['max', 'perSession']);
const STEP_MS = 450;

/** 机器设置的保存：每一项各存各的（store 的 saveSetting）。改动当帧生效，不禁用任何控件，没确认时也能接着改；
    sync 是每一项的保存状态，画在那个控件旁边：保存中（转圈，晚一点才出现）→ 已保存（打勾后淡出）；
    被拒绝时值回到原处、那一行回弹，并提示 */
export function useSaver(name: string) {
  const { sync, track } = useSync();
  const save: Save = p => {
    const f = fieldOf(p);
    const { env, run, resume, ...fields } = p;
    const done = saveSetting(
      `${hostKey(name)}:${f}`,
      {
        ws: ws => ({
          ...ws,
          hosts: ws.hosts?.map(x =>
            x.name === name
              ? {
                  ...x,
                  ...fields,
                  agents: env || run || resume ? x.agents.map(a => agentPatch(a, p)) : x.agents,
                  sync: touchesConfig(p) ? syncing(x) : x.sync,
                }
              : x,
          ),
        }),
      },
      () => api.updateHost(name, p),
      STEPPED.has(f) ? STEP_MS : 0,
    );
    track(f, done);
  };
  return { save, sync };
}

/** 步进器：数字当帧变，往上加从下面滚进来、往下减从上面滚进来 */
export function Stepper({ value, min = 1, max = 8, label, onChange }: { value: number; min?: number; max?: number; label: string; onChange: (v: number) => void }) {
  const prev = useRef(value);
  const up = value >= prev.current;
  useEffect(() => {
    prev.current = value;
  }, [value]);
  return (
    <span className="hp-stepper" role="group" aria-label={label}>
      <button type="button" aria-label="减少" disabled={value <= min} onClick={() => onChange(value - 1)}>
        −
      </button>
      <b aria-live="polite">
        <span key={value} className="hp-num" style={{ ['--roll' as string]: up ? '80%' : '-80%' }}>
          {value}
        </span>
      </b>
      <button type="button" aria-label="增加" disabled={value >= max} onClick={() => onChange(value + 1)}>
        +
      </button>
    </span>
  );
}
