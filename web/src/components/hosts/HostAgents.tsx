/* ==========================================================================
   机器详情「agent」页：每个 agent 一张卡，控件直接摆出来
   - 顶部一句：写进这台机器上 agent 自己的配置文件……；右边「自检 x 前」「全部自检」
   - 卡片：标题行（图标、名称、默认、版本、右上角状态：正常 · 自检 x 前 / 自检没过 / 配置没写进去 / 缺 board-skills，有问题描边变黄）、
     写入状态一行（已写入 / 正在写入 / 不改 / 机器离线 / 写入失败 + 重试）、原因一行（+ 再试一次）、
     启动环境（每项带版本，没找到的置灰）、模型（一个下拉：不改 / 供应商 · 模型；agent 认几种接口、供应商也给了几种时多一个接口；强度）、
     下面一句写用哪种接口连哪个地址；设为默认、自检这一个
   - 没找到的 agent 卡片置灰，写「这台机器上没找到」
   写操作：模型、启动环境、默认 agent 是设置项（当帧生效，旁边转圈）；自检、重试要等服务端（按钮转圈、禁用直到返回）
   ========================================================================== */
import { useRef, useState } from 'react';
import { api } from '../../lib/api';
import { agentHealth, configLine, lastTest } from '../../lib/hostDetail';
import { envName, missWord, selftestBusy, selftestLive, selftestState, selftestWhole, since } from '../../lib/hosts';
import { interfaceName, metaOf, protocolChoices, providersFor, runPick, writableEfforts } from '../../lib/providers';
import type { HostAgent, HostInfo, ProviderView } from '../../lib/types';
import { copyText, cx } from '../../lib/util';
import { optimistic } from '../../store/data';
import { toast, toastError } from '../../store/ui';
import { Select, Sync, Tip, type SyncState } from '../bits';
import { Icon } from '../Icon';
import { AgentIcon } from './bits';
import type { Save } from './Saver';

type SyncMap = Record<string, SyncState | undefined>;

/** 只等服务端的按钮（自检、重试）：转圈、禁用直到返回。按 key 记：几个 agent 的「自检这一个」可以同时点（#487） */
function useAsk() {
  const [busy, setBusy] = useState<ReadonlySet<string>>(() => new Set());
  // 同一帧连点两下时 state 还没更新：用 ref 挡住同一个 key 的第二次
  const live = useRef(new Set<string>());
  const run = async (key: string, f: () => Promise<unknown>) => {
    if (live.current.has(key)) return;
    live.current.add(key);
    setBusy(new Set(live.current));
    try {
      await optimistic(null, f);
    } catch (e) {
      toastError(e);
    } finally {
      live.current.delete(key);
      setBusy(new Set(live.current));
    }
  };
  return { any: busy.size > 0, is: (key: string) => busy.has(key), run };
}

export function HostAgents({ h, view, save, sync, now }: { h: HostInfo; view: ProviderView | null; save: Save; sync: SyncMap; now: number }) {
  const ask = useAsk();
  const tested = lastTest(h);
  const testing = selftestBusy(h, now);
  const whole = selftestWhole(h, now);
  const state = selftestState(h, now);
  const offline = !h.online;
  return (
    <div className="hd-agents">
      <div className="hd-abar">
        <span className="hp-note">写进这台机器上 agent 自己的配置文件：board host 启动时写一次，服务器上一改就同步。有人在用的电脑选「不改」，Board 就不碰它。</span>
        <span className="hd-test">
          {state?.tone === 'busy' && state.agents.length > 0 ? (
            <span>正在自检 {state.agents.map(id => h.agents.find(x => x.id === id)?.name ?? id).join('、')}</span>
          ) : (
            tested > 0 && <span>自检 {since(tested, now)}</span>
          )}
          <Tip text={offline ? '机器离线' : h.selftest?.able === false ? `board ${h.version} 不会自检：升级后再试` : '问每个 agent 一句、检查每个项目能不能推送；会消耗 agent 的额度'}>
            <span>
              <button
                className={cx('btn sm', (ask.is('all') || whole) && 'is-busy')}
                type="button"
                disabled={ask.any || testing || offline || h.selftest?.able === false}
                onClick={() => void ask.run('all', () => api.selfTest(h.name))}
              >
                <Icon name="check" />
                {whole ? '正在自检' : '全部自检'}
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
  // 正在查这个 agent（转圈）；整台机器在查时也不能单查它（禁用）。别的 agent 在查不影响它（#487）
  const mine = ask.is(`one:${a.id}`) || ask.is(`test:${a.id}`) || selftestLive(h, now).some(x => x.agent === a.id);
  const testing = mine || ask.is('all') || selftestBusy(h, now, a.id);
  const failed = [`env:${a.id}`, `run:${a.id}`, `resume:${a.id}`].some(k => sync[k] === 'failed');
  return (
    <article className={cx('hd-agent', !a.found && 'missing', a.found && !hr.ok && 'warn', failed && 'hp-rollback')}>
      <header>
        <AgentIcon id={a.id} />
        <b>{a.name}</b>
        {def && <span className="ha-def">默认</span>}
        <span className="hd-ver">{a.found ? a.version || '找到了' : a.missing ? [a.version, missWord(a)].filter(Boolean).join(' · ') : '这台机器上没找到'}</span>
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
      <ManageAgent h={h} a={a} now={now} ask={ask} />
      {a.found && meta && (
        <div className={cx('hd-conf', line.tone)}>
          <Icon name={line.tone === 'ok' ? 'checkCircle' : line.tone === 'bad' ? 'alert' : line.tone === 'busy' ? 'renew' : 'info'} />
          <span>{line.text}</span>
          {line.retry && (
            <button className={cx('btn sm', ask.is(`sync:${a.id}`) && 'is-busy')} type="button" disabled={ask.any || !h.online} onClick={() => void ask.run(`sync:${a.id}`, () => api.syncHost(h.name))}>
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
            <button className={cx('btn sm', ask.is(`fix:${a.id}`) && 'is-busy')} type="button" disabled={ask.any || !h.online} onClick={() => void ask.run(`fix:${a.id}`, () => api.syncHost(h.name))}>
              {hr.actLabel}
            </button>
          ) : (
            hr.act === 'selftest' && (
              <button
                className={cx('btn sm', mine && 'is-busy')}
                type="button"
                disabled={testing || !h.online}
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
      {a.found && <Editor h={h} a={a} view={view} save={save} sync={sync} def={def} testing={testing} mine={mine} ask={ask} />}
    </article>
  );
}

/** agent 的安装方式、最新版本、安装/更新按钮。失败的留在卡片上，可以重试。 */
function ManageAgent({ h, a, now, ask }: { h: HostInfo; a: HostAgent; now: number; ask: ReturnType<typeof useAsk> }) {
  const m = a.manage;
  if (!m) return null;
  const queued = h.ask?.agent === a.id;
  const running = m.state === 'running' || queued;
  const key = `agent:${a.id}`;
  const disabled = ask.any || !h.online || running;
  const install = () => void ask.run(key, () => api.askAgent(h.name, 'agent-install', a.id, m.latest_version));
  const update = () => void ask.run(key, () => api.askAgent(h.name, 'agent-update', a.id, m.latest_version));
  const copy = async () => {
    if (!m.install_command) return;
    const ok = await copyText(m.install_command);
    toast({ type: ok ? 'success' : 'error', title: ok ? '已复制安装命令' : '复制失败' });
  };
  // 装了 agent、缺 ACP 入口要的命令（Node.js）：不是安装方式的事，单独一行说缺什么、怎么办（#458）
  if (m.method === 'needs')
    return (
      <div className="hd-why">
        <span>{m.reason || `缺 ${a.missing ?? ''}`}</span>
      </div>
    );
  return (
    <div className={cx('hd-manage', m.state === 'failed' && 'hp-rollback')}>
      <div className="hd-conf muted">
        <Icon name={m.update_available ? 'download' : 'info'} />
        <span>
          安装方式：{m.method_label || '未知'}
          {m.latest_version ? ` · 最新 ${m.latest_version}` : ''}
          {m.checked_at ? ` · 检查于 ${since(m.checked_at, now)}` : ''}
        </span>
        {running && <span className="ha-hint">正在{a.found ? '更新' : '安装'}…</span>}
        {!running && m.update_available && m.updatable && (
          <button className={cx('btn sm', ask.is(key) && 'is-busy')} type="button" disabled={disabled} onClick={update}>
            <Icon name="download" />
            更新到 {m.latest_version}
          </button>
        )}
      </div>
      {!a.found && m.install_command && (
        <div className="hd-manage-cmd">
          <code>{m.install_command}</code>
          <button className="btn sm ghost" type="button" onClick={() => void copy()}>
            <Icon name="copy" />
            复制
          </button>
          {m.installable && !running && (
            <button className={cx('btn sm brand', ask.is(key) && 'is-busy')} type="button" disabled={disabled} onClick={install}>
              <Icon name="download" />
              安装
            </button>
          )}
        </div>
      )}
      {m.reason && !running && <p className="hp-note">{m.reason}</p>}
      {m.error && !running && (
        <div className="hd-why">
          <span>上一次失败：{m.error}</span>
          {m.updatable && m.latest_version && (
            <button className={cx('btn sm', ask.is(key) && 'is-busy')} type="button" disabled={disabled} onClick={update}>
              重试
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** 下拉里一项「供应商 · 模型」的值 */
const pickValue = (provider: string, model: string) => `${provider}\u0001${model}`;

/** 一个 agent 的控件：启动环境、模型（不改 / 供应商 · 模型，接口、强度）、设为默认、自检这一个 */
function Editor({
  h,
  a,
  view,
  save,
  sync,
  def,
  testing,
  mine,
  ask,
}: {
  h: HostInfo;
  a: HostAgent;
  view: ProviderView | null;
  save: Save;
  sync: SyncMap;
  def: boolean;
  /** 不能单查它：它或整台机器正在自检 */
  testing: boolean;
  /** 正在查它（转圈） */
  mine: boolean;
  ask: ReturnType<typeof useAsk>;
}) {
  const meta = metaOf(view, a.id);
  const p = runPick(view, a);
  const off = p.mode === 'off';
  const fits = providersFor(view, a.id);
  const prov = view?.providers.find(x => x.name === p.provider);
  const efforts = writableEfforts(meta, prov, p.model);
  const protos = off ? [] : protocolChoices(view, a.id, p.provider);
  const name = (id: string) => envName(h, id) ?? id;
  /** 指定一组。接口：带了用它；没带时同一个供应商留着原来的（新供应商不给那种了就回到第一个对得上的） */
  const set = (provider: string, model: string, effort: string, protocol?: string) => {
    const keep = protocol ?? (provider === a.run_provider && protocolChoices(view, a.id, provider).some(e => e.interface === a.run_protocol) ? (a.run_protocol ?? '') : '');
    save({ run: { agent: a.id, mode: 'own', provider, model, effort, protocol: keep } });
  };
  /** 换模型时强度：新模型也支持就留着 */
  const keepEffort = (provider: string, model: string) => (writableEfforts(meta, view?.providers.find(x => x.name === provider), model).includes(p.effort) ? p.effort : '');
  const hint = off
    ? `Board 不碰它的配置文件${a.model ? `：本机现在是 ${a.model}${a.effort ? ` · 强度 ${a.effort}` : ''}` : ''}`
    : p.gone
      ? `供应商 ${p.provider} 在服务器上已经删了：换一个`
      : p.ep
        ? `用 ${interfaceName(p.ep.interface)} 接口连 ${p.ep.url}，写进这台机器上它的配置文件`
        : `${p.provider} 没有 ${a.name} 认的接口：换一个`;
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
              options={a.envs.map(c => ({ value: c.env, label: `${name(c.env)} · ${c.found ? c.version || '找到了' : missWord(c)}`, disabled: !c.found }))}
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
              <Tip text={!fits.length && off ? '服务器上还没有接口对得上的供应商：先在「Agent → 模型供应商」里加一个' : ''}>
                <span>
                  <Select
                    className="ha-sel wide"
                    value={off ? '' : pickValue(p.provider, p.model)}
                    placeholder={p.gone ? `${p.provider} · ${p.model}（已删）` : undefined}
                    options={[{ value: '', label: '不改' }, ...fits.flatMap(x => x.models.map(m => ({ value: pickValue(x.name, m), label: `${x.name} · ${m}` })))]}
                    onChange={v => {
                      if (!v) return save({ run: { agent: a.id, mode: 'off' } });
                      const [provider, model] = v.split('\u0001');
                      set(provider, model, keepEffort(provider, model));
                    }}
                    aria-label={`${a.name} 用哪个模型`}
                  />
                </span>
              </Tip>
              {protos.length > 1 && (
                <Tip text={`${a.name} 认这几种，${p.provider} 也都给了：选用哪种`}>
                  <span>
                    <Select
                      className="ha-sel"
                      value={p.ep?.interface ?? ''}
                      options={protos.map(e => ({ value: e.interface, label: `接口 ${interfaceName(e.interface)}` }))}
                      onChange={v => set(p.provider, p.model, p.effort, v)}
                      aria-label={`${a.name} 用哪种接口`}
                    />
                  </span>
                </Tip>
              )}
              {!off && (
                <Tip text={efforts.length ? '只列这个模型支持、这个 agent 也能写的档位' : '供应商里没给这个模型标强度：不能选'}>
                  <span>
                    <Select
                      className="ha-sel sm"
                      value={efforts.includes(p.effort) ? p.effort : ''}
                      disabled={!efforts.length}
                      options={[{ value: '', label: '强度默认' }, ...efforts.map(e => ({ value: e, label: `强度 ${e}` }))]}
                      onChange={v => set(p.provider, p.model, v)}
                      aria-label={`${a.name} 的强度`}
                    />
                  </span>
                </Tip>
              )}
            </span>
            <span className="ha-hint">{hint}</span>
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
          className={cx('btn sm ghost', mine && 'is-busy')}
          type="button"
          disabled={testing || !h.online || h.selftest?.able === false}
          onClick={() => void ask.run(`one:${a.id}`, () => api.selfTest(h.name, a.id))}
        >
          <Icon name="check" />
          自检这一个
        </button>
      </span>
    </div>
  );
}
