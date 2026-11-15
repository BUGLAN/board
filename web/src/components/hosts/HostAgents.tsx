/* ==========================================================================
   机器详情「agent」页：每个 agent 一张卡，控件直接摆出来
   - 顶部一句：写进这台机器上 agent 自己的配置文件……；右边「自检 x 前」「全部自检」
   - 卡片：标题行（图标、名称、默认、版本、右上角状态：正常 · 自检 x 前 / 自检没过 / 配置没写进去 / 缺 board-skills，有问题描边变黄）、
     写入状态一行（已写入 / 正在写入 / 不改 / 服务器上没给它配 / 机器离线 / 写入失败 + 重试）、原因一行（+ 再试一次）、
     启动环境（每项带版本，没找到的置灰）、模型（用服务器的 / 不改 / 单独配置：供应商、模型、强度）、设为默认、自检这一个
   - 没找到的 agent 卡片置灰，写「这台机器上没找到」
   写操作：模型、启动环境、默认 agent 是设置项（当帧生效，旁边转圈）；自检、重试要等服务端（按钮转圈、禁用直到返回）
   ========================================================================== */
import { useState } from 'react';
import { api } from '../../lib/api';
import { agentHealth, configLine, lastTest } from '../../lib/hostDetail';
import { envName, selftestBusy, since } from '../../lib/hosts';
import { metaOf, providersFor, RUN_MODES, runPick, runWords, writableEfforts } from '../../lib/providers';
import type { HostAgent, HostInfo, ProviderView, RunMode } from '../../lib/types';
import { cx } from '../../lib/util';
import { optimistic } from '../../store/data';
import { toastError } from '../../store/ui';
import { Segmented, Select, Sync, Tip, type SyncState } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import type { Save } from './Saver';

type SyncMap = Record<string, SyncState | undefined>;

/** 只等服务端的按钮（自检、重试）：转圈、禁用直到返回 */
function useAsk() {
  const [busy, setBusy] = useState<string | null>(null);
  const run = async (key: string, f: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(key);
    try {
      await optimistic(null, f);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  };
  return { busy, run };
}

export function HostAgents({ h, view, save, sync, now }: { h: HostInfo; view: ProviderView | null; save: Save; sync: SyncMap; now: number }) {
  const ask = useAsk();
  const tested = lastTest(h);
  const testing = selftestBusy(h, now);
  const offline = !h.online;
  return (
    <div className="hd-agents">
      <div className="hd-abar">
        <span className="hp-note">写进这台机器上 agent 自己的配置文件：board host 启动时写一次，服务器上一改就同步。有人在用的电脑选「不改」，Board 就不碰它。</span>
        <span className="hd-test">
          {tested > 0 && <span>自检 {since(tested, now)}</span>}
          <Tip text={offline ? '机器离线' : h.selftest?.able === false ? `board ${h.version} 不会自检：升级后再试` : '问每个 agent 一句、检查每个项目能不能推送；会消耗 agent 的额度'}>
            <span>
              <button
                className={cx('btn sm', (ask.busy === 'all' || (testing && !h.selftest?.ask?.agent)) && 'is-busy')}
                type="button"
                disabled={!!ask.busy || testing || offline || h.selftest?.able === false}
                onClick={() => void ask.run('all', () => api.selfTest(h.name))}
              >
                <Icon name="check" />
                {testing && !h.selftest?.ask?.agent ? '正在自检' : '全部自检'}
              </button>
            </span>
          </Tip>
        </span>
      </div>
      {h.agentSet && (
        <p className="hp-note hp-warn-line">
          <Icon name="alert" />
          设置的默认 agent「{h.agentSet}」这台机器上找不到，不生效{h.agent ? `：用的是 ${h.agents.find(a => a.id === h.agent)?.name ?? h.agent}` : ''}
        </p>
      )}
      {h.agents.map(a => (
        <AgentCard key={a.id} h={h} a={a} view={view} save={save} sync={sync} now={now} ask={ask} />
      ))}
      {h.agents.length === 0 && <p className="hp-note">board host 没有上报 agent</p>}
    </div>
  );
}

function AgentCard({ h, a, view, save, sync, now, ask }: { h: HostInfo; a: HostAgent; view: ProviderView | null; save: Save; sync: SyncMap; now: number; ask: ReturnType<typeof useAsk> }) {
  const hr = agentHealth(h, a);
  const meta = metaOf(view, a.id);
  const def = a.id === h.agent;
  const pick = runPick(view, a);
  const line = configLine(h, a, view, now, pick);
  const testing = selftestBusy(h, now) && (!h.selftest?.ask?.agent || h.selftest.ask.agent === a.id);
  const failed = [`env:${a.id}`, `run:${a.id}`, `resume:${a.id}`].some(k => sync[k] === 'failed');
  return (
    <article className={cx('hd-agent', !a.found && 'missing', a.found && !hr.ok && 'warn', failed && 'hp-rollback')}>
      <header>
        <AgentIcon id={a.id} />
        <b>{a.name}</b>
        {def && <span className="ha-def">默认</span>}
        <span className="hd-ver">{a.found ? a.version || '找到了' : '这台机器上没找到'}</span>
        {a.found &&
          (hr.ok ? (
            <span className="hd-ok">
              <Icon name="checkCircle" />
              {hr.at ? `正常 · 自检 ${since(hr.at, now)}` : '正常'}
            </span>
          ) : (
            <span className="hp-skill">
              <Icon name="alert" />
              {hr.chip}
            </span>
          ))}
      </header>
      {a.found && meta && (
        <div className={cx('hd-conf', line.tone)}>
          <Icon name={line.tone === 'ok' ? 'checkCircle' : line.tone === 'bad' ? 'alert' : line.tone === 'busy' ? 'renew' : 'info'} />
          <span>{line.text}</span>
          {line.retry && (
            <button className={cx('btn sm', ask.busy === `sync:${a.id}` && 'is-busy')} type="button" disabled={!!ask.busy || !h.online} onClick={() => void ask.run(`sync:${a.id}`, () => api.syncHost(h.name))}>
              重试
            </button>
          )}
        </div>
      )}
      {a.found && !meta && <div className="hd-conf muted">
          <Icon name="info" />
          <span>配置文件里新加的 agent：模型在它自己的配置里，Board 不写</span>
        </div>}
      {a.found && hr.why && (
        <div className="hd-why">
          <span>{hr.why}</span>
          {hr.act === 'sync' ? (
            <button className={cx('btn sm', ask.busy === `fix:${a.id}` && 'is-busy')} type="button" disabled={!!ask.busy || !h.online} onClick={() => void ask.run(`fix:${a.id}`, () => api.syncHost(h.name))}>
              {hr.actLabel}
            </button>
          ) : (
            hr.act === 'selftest' && (
              <button
                className={cx('btn sm', (ask.busy === `test:${a.id}` || testing) && 'is-busy')}
                type="button"
                disabled={!!ask.busy || testing || !h.online}
                onClick={() => void ask.run(`test:${a.id}`, () => api.selfTest(h.name, a.id))}
              >
                {hr.actLabel}
              </button>
            )
          )}
        </div>
      )}
      {a.tripped && (
        <div className="hd-why">
          <span>连续 3 次异常退出，已经暂停接单：不再给它派新的卡。看「概况 → 最近」里那几次的原因，修好后恢复。</span>
          <button className={cx('btn sm', sync[`resume:${a.id}`] === 'saving' && 'is-busy')} type="button" onClick={() => save({ resume: a.id })}>
            恢复
          </button>
        </div>
      )}
      {a.found && <Editor h={h} a={a} view={view} save={save} sync={sync} def={def} testing={testing} ask={ask} />}
    </article>
  );
}

/** 一个 agent 的控件：启动环境、模型（用服务器的 / 不改 / 单独配置）、设为默认、自检这一个 */
function Editor({ h, a, view, save, sync, def, testing, ask }: { h: HostInfo; a: HostAgent; view: ProviderView | null; save: Save; sync: SyncMap; def: boolean; testing: boolean; ask: ReturnType<typeof useAsk> }) {
  const meta = metaOf(view, a.id);
  const p = runPick(view, a);
  const fits = providersFor(view, a.id);
  const prov = view?.providers.find(x => x.name === p.provider);
  const efforts = writableEfforts(meta, prov, p.model);
  const name = (id: string) => envName(h, id) ?? id;
  const setMode = (m: RunMode) => {
    if (m === p.mode) return;
    if (m !== 'own') return save({ run: { agent: a.id, mode: m } });
    // 单独配置：先给一个接口对得上的供应商（服务器上给它配的那个，没有时第一个），之后再改
    const shared = view?.agents.find(x => x.agent === a.id)?.provider;
    const first = fits.find(x => x.name === shared) ?? fits[0];
    if (first) save({ run: { agent: a.id, mode: 'own', provider: first.name, model: '', effort: '' } });
  };
  return (
    <div className="hd-edit">
      <span className="hd-k">启动环境</span>
      <span className="hd-v hd-col">
        <span className="hp-ctl">
          <Sync s={sync[`env:${a.id}`]} />
          {a.envs?.length ? (
            <Select
              className="ha-sel wide"
              value={a.env ?? ''}
              options={a.envs.map(c => ({ value: c.env, label: `${name(c.env)} · ${c.found ? c.version || '找到了' : '没找到'}`, disabled: !c.found }))}
              onChange={v => save({ env: { agent: a.id, env: v } })}
              aria-label={`${a.name} 的启动环境`}
            />
          ) : (
            <span className="ha-hint">直接启动（board {h.version || '?'} 不上报启动环境）</span>
          )}
        </span>
        {a.run_env && a.env && a.run_env !== a.env && (
          <span className="ha-hint hd-warn">
            <Icon name="alert" />
            设置的「{name(a.run_env)}」这台机器上没有，不生效：用的是 {name(a.env)}
          </span>
        )}
      </span>

      {meta && (
        <>
          <span className="hd-k">模型</span>
          <span className="hd-v hd-col">
            <span className="hp-ctl">
              <Sync s={sync[`run:${a.id}`]} />
              <Tip text={!fits.length && p.mode !== 'own' ? '服务器上还没有接口对得上的供应商：不能单独配置' : ''}>
                <span>
                  <Segmented<RunMode> value={p.mode} options={RUN_MODES.map(([k, v]) => [k, v, k === 'own' && !fits.length && p.mode !== 'own'] as [RunMode, string, boolean])} onChange={setMode} />
                </span>
              </Tip>
            </span>
            {p.mode === 'own' && (
              <span className="hd-own">
                <Select
                  className="ha-sel"
                  value={p.provider}
                  placeholder={p.gone ? `${p.provider}（已删）` : '选供应商'}
                  options={fits.map(x => ({ value: x.name, label: x.name }))}
                  onChange={v => save({ run: { agent: a.id, mode: 'own', provider: v, model: '', effort: '' } })}
                  aria-label="供应商"
                />
                <Select
                  className="ha-sel"
                  value={p.model}
                  options={(prov?.models ?? []).map(m => ({ value: m, label: m }))}
                  onChange={v => save({ run: { agent: a.id, mode: 'own', provider: p.provider, model: v, effort: writableEfforts(meta, prov, v).includes(p.effort) ? p.effort : '' } })}
                  aria-label="模型"
                />
                <Tip text={efforts.length ? '只列这个模型支持、这个 agent 也能写的档位' : '供应商里没给这个模型标强度：不能选'}>
                  <span>
                    <Select
                      className="ha-sel sm"
                      value={efforts.includes(p.effort) ? p.effort : ''}
                      disabled={!efforts.length}
                      options={[{ value: '', label: '强度默认' }, ...efforts.map(e => ({ value: e, label: `强度 ${e}` }))]}
                      onChange={v => save({ run: { agent: a.id, mode: 'own', provider: p.provider, model: p.model, effort: v } })}
                      aria-label="强度"
                    />
                  </span>
                </Tip>
              </span>
            )}
            <span className="ha-hint">
              {p.mode === 'off'
                ? 'Board 不碰它的配置文件'
                : p.mode === 'own'
                  ? p.gone
                    ? `供应商 ${p.provider} 在服务器上已经删了：换一个`
                    : '只给这台机器：写进它的配置文件，服务器上的不管'
                  : p.none
                    ? '服务器上没给它配供应商：不写（在共用配置里改）'
                    : `服务器上：${runWords(p)}（在共用配置里改）`}
            </span>
            {p.warn && (
              <span className="ha-hint hd-warn">
                <Icon name="alert" />
                {p.warn}
              </span>
            )}
          </span>
        </>
      )}

      <span className="hd-k" />
      <span className="hd-v hd-acts">
        {def ? (
          <span className="ha-hint">默认 agent：手动派发时预先选中</span>
        ) : (
          <span className="hp-ctl">
            <Sync s={sync.agent} />
            <button className="btn sm ghost" type="button" onClick={() => save({ agent: a.id })}>
              设为默认
            </button>
          </span>
        )}
        <button
          className={cx('btn sm ghost', (ask.busy === `one:${a.id}` || (testing && h.selftest?.ask?.agent === a.id)) && 'is-busy')}
          type="button"
          disabled={!!ask.busy || testing || !h.online || h.selftest?.able === false}
          onClick={() => void ask.run(`one:${a.id}`, () => api.selfTest(h.name, a.id))}
        >
          <Icon name="check" />
          自检这一个
        </button>
      </span>
    </div>
  );
}
