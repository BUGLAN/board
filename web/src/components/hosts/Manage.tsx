/* ==========================================================================
   机器详情「维护」（PRD §9.1「远程管理」、§10.12）：在别的设备上维护这台机器
   - 重新检查：board host 重新找启动环境、本机 agent、board-skills，立即上报
   - 升级到 <服务端版本>：这台机器比服务端旧时才有；先暂停接单，等在跑的派发结束（或「立即升级」）再升级
   - 重启 board host：二次确认；在跑的派发按「人停止」结束
   - 排空：暂停接单，等在跑的结束后显示「已排空」
   - 令牌：这台机器用的令牌（名称、开头几位、最近使用）
   请求要等服务端排进去才有记录：按钮进行中（转圈、禁用）直到返回（optimistic(null, …)）；
   请求本身的进展（等 N 个派发结束、正在重启）按服务端推回来的 ask 画
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { hostLoad, since } from '../../lib/hosts';
import type { HostInfo } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic, useData } from '../../store/data';
import { confirm, toast, toastError } from '../../store/ui';
import { Tip } from '../bits';
import { Icon } from '../Icon';

type Kind = 'recheck' | 'restart' | 'upgrade' | 'drain' | 'cancel';

/** 版本号 a 比 b 旧（和服务端 version_lt 一致） */
export function versionLt(a: string, b: string) {
  const parts = (v: string) => {
    const p = v.trim().replace(/^v/, '').split(/[.+-]/).slice(0, 3).map(x => Number.parseInt(x, 10) || 0);
    while (p.length < 3) p.push(0);
    return p;
  };
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] < y[i];
  return false;
}

/** 请求的进展（一行字）：等几个派发结束、正在重启、已排空……；没有请求也不在排空时为空 */
export function askState(h: HostInfo, running: number): { text: string; tone: 'busy' | 'ok' | 'warn' } | null {
  const a = h.ask;
  if (a?.kind === 'recheck') return { text: '正在重新检查：等 board host 重新找启动环境、agent、board-skills', tone: 'busy' };
  if (a?.kind === 'restart') return { text: running ? `等 ${running} 个在跑的派发停下后重启 board host` : '正在重启 board host', tone: 'busy' };
  if (a?.kind === 'upgrade') {
    const to = a.to ?? '新版本';
    return { text: running ? `等 ${running} 个派发结束后升级到 ${to}` : `正在升级到 ${to}`, tone: 'busy' };
  }
  if (h.drain) return running ? { text: `排空中：等 ${running} 个在跑的派发结束`, tone: 'warn' } : { text: '已排空，可以关机或维护', tone: 'ok' };
  return null;
}

export function ManageSection({ h, server, i }: { h: HostInfo; server: string | null; i: number }) {
  const ws = useData(s => s.ws);
  const running = hostLoad(ws, h).running.length;
  const [busy, setBusy] = useState<Kind | null>(null);
  const old = !!server && !!h.version && versionLt(h.version, server);
  const state = askState(h, running);
  const waitingUpgrade = h.ask?.kind === 'upgrade' && !h.ask.now && running > 0;
  // 旧版服务端没有远程管理（不发 ask、ops）：不显示这一节
  if (h.ops === undefined) return null;

  const send = async (kind: Kind, now = false, done?: string) => {
    if (busy) return;
    setBusy(kind);
    try {
      await optimistic(null, () => api.askHost(h.name, kind, now));
      if (done) toast({ type: 'success', title: done });
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  const restart = async () => {
    const yes = await confirm({
      title: `重启 ${h.name} 上的 board host？`,
      text: running
        ? `正在跑的 ${running} 个派发会按「人停止」结束（卡片移到需要人处理的状态）。board host 重新启动后重新读配置、重新检查，一般 30 秒内重新上线。`
        : 'board host 重新启动后重新读配置、重新检查，一般 30 秒内重新上线。',
      ok: '重启',
      icon: 'alert',
      tone: 'warn',
    });
    if (yes) void send('restart', false, `已通知 ${h.name} 重启 board host`);
  };
  const upgradeNow = async () => {
    const yes = await confirm({
      title: `立即升级 ${h.name}？`,
      text: `正在跑的 ${running} 个派发会按「人停止」结束（卡片移到需要人处理的状态），然后马上升级到 ${server}。`,
      ok: '立即升级',
      icon: 'alert',
      tone: 'warn',
    });
    if (yes) void send('upgrade', true);
  };
  const btn = (kind: Kind, label: string, icon: Parameters<typeof Icon>[0]['name'], act: () => void, tip: string, disabled = false) => (
    <Tip text={tip}>
      <span>
        <button className={cx('btn sm', busy === kind && 'is-busy')} type="button" disabled={!!busy || disabled} aria-busy={busy === kind || undefined} onClick={act}>
          <Icon name={icon} />
          {label}
        </button>
      </span>
    </Tip>
  );
  const offline = !h.online;
  const pending = !!h.ask;
  const t = h.token;
  return (
    <section className="d-section hp-manage" style={{ ['--i' as string]: i }}>
      <div className="d-label">
        维护<span className="act">在这里远程维护这台机器，不用登录过去</span>
      </div>
      <div className="hp-manage-acts">
        {btn('recheck', '重新检查', 'renew', () => void send('recheck', false, `已通知 ${h.name} 重新检查`), offline ? '机器离线' : '重新找启动环境、本机 agent、board-skills，在跑的派发不受影响', offline || pending)}
        {old &&
          btn(
            'upgrade',
            `升级到 ${server}`,
            'download',
            () => void send('upgrade', false, `${h.name} 会在在跑的派发结束后升级`),
            offline ? '机器离线' : `这台机器是 ${h.version}：先暂停接单，等在跑的派发结束再升级；升级完恢复接单`,
            offline || pending,
          )}
        {btn('restart', '重启 board host', 'replay', () => void restart(), offline ? '机器离线' : '结束本机由它启动的全部 agent，再重新启动', offline || pending)}
        {btn('drain', '排空', 'pause', () => void send('drain', false), '暂停接单，等在跑的派发结束：适合关机、维护之前', !!h.drain)}
      </div>
      {state && (
        <div className={cx('hp-ask', state.tone)} role="status">
          {state.tone === 'busy' && <i className="hp-ask-spin" />}
          {state.tone === 'ok' && <Icon name="check" />}
          <span>
            {state.text}
            {h.ask && (
              <em>
                {' '}
                · {h.ask.by} {since(h.ask.at, Date.now())}请求
              </em>
            )}
          </span>
          {waitingUpgrade && (
            <button className="btn sm" type="button" disabled={!!busy} onClick={() => void upgradeNow()}>
              立即升级
            </button>
          )}
          {h.ask && (
            <button className={cx('btn sm ghost', busy === 'cancel' && 'is-busy')} type="button" disabled={!!busy} onClick={() => void send('cancel')}>
              取消
            </button>
          )}
        </div>
      )}
      <div className="hp-token">
        <span className="hp-token-k">令牌</span>
        {t ? (
          <span className="hp-token-v">
            <b>{t.name}</b>
            <code>{t.hint}</code>
            <em>{t.lastUsedAt ? `最近使用 ${since(t.lastUsedAt, Date.now())}` : '还没用过'}</em>
            {!!t.shared?.length && <em>· 机器 {t.shared.join('、')} 也在用</em>}
          </span>
        ) : (
          <span className="hp-token-v">
            <em>看不到：还没上报过，或令牌已经吊销</em>
          </span>
        )}
      </div>
    </section>
  );
}
