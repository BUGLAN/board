/* ==========================================================================
   定时建卡（PRD §4「白板设置」，白板设置里的一节）：到点按模板在这块白板上建一张卡，由定时领取或人派发去做，
   也可以直接派给一台机器（例如定期让某台机器更新代码）
   - 每条一行：开关、名称、时间（和时区）、放到哪、派给哪台机器、标签；下面一行写「下次：<时间>」和最近一次的结果
     （「11-12 09:00 建了 #263 · 派发到 mbp · Claude Code」或「11-12 09:00 跳过：上一张 #250 还没完成」）；右边「现在建一张」、编辑、删除
   - 编辑（和新建）原地展开：名称、卡片标题、描述（和新建卡片同一套：粘贴截图、预览）、标签、放到哪、派给机器（和 agent）、
     时间（SchedulePick，和机器设置的「定时领取」同一个组件）；时区取保存时浏览器的
   写操作（AGENTS.md「界面写操作」）：新建、删除、现在建一张要等服务端返回（optimistic(null, …)，按钮转圈并禁用）；
   开关是设置项（saveSetting，不锁）；改模板用乐观补丁（cardCronPatch + cardCronKey），当帧生效，失败时带着草稿重新打开
   ========================================================================== */
import { useRef, useState, type SetStateAction } from 'react';
import { openCard } from '../lib/actions';
import { api } from '../lib/api';
import { stillUploading, useAttach } from '../lib/attach';
import { buildCron, describeCron, normalizeCron, parseSchedule, type Schedule } from '../lib/cron';
import { boardOf, defaultAgent, envName, hostLoad, hostsOf } from '../lib/hosts';
import { boardFlow } from '../lib/schema';
import type { CardCron, CardCronInput, HostInfo, Label } from '../lib/types';
import { cx } from '../lib/util';
import { cardCronKey, cardCronPatch, optimistic, saveSetting, useBusy, useCurrent, useData } from '../store/data';
import { confirm, openBoardSettings, openPicker, toast, toastError } from '../store/ui';
import { DescTabs, InfoTip, LabelChip, Markdown, Segmented, Sync, useNow, useSync } from './bits';
import { AgentChip, Dot } from './hosts/bits';
import { Icon } from './Icon';
import { DEFAULT_SCHEDULE, EVERY_MINUTES, SchedulePick } from './SchedulePick';

const NONE: CardCron[] = [];
const NO_LABELS: Label[] = [];
/** 每块白板最多几条（和服务端一致） */
const MAX = 20;
/** 定时建卡的「每隔几分钟」多一个 1 分钟 */
const MINUTES = [1, ...EVERY_MINUTES];

/** 这台电脑的时区（IANA 名称），保存时带上 */
export const browserTz = () => Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';

const pad = (n: number) => String(n).padStart(2, '0');
/** 「11-12 09:00」（按这台电脑的时间） */
const when = (t: number) => {
  const d = new Date(t);
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

interface Draft {
  name: string;
  title: string;
  body: string;
  labels: number[];
  place: 'home' | 'inbox';
  /** 派给哪台机器；不派发时为空 */
  host: string | null;
  /** 用哪个 agent；空表示那台机器的默认 agent */
  agent: string | null;
  spec: Schedule;
  /** 原来的表达式不是选择组件能生成的（手写的）：不动它就原样保存 */
  raw: string | null;
}

const draftOf = (c?: CardCron): Draft => {
  const spec = c ? parseSchedule(c.cron) : null;
  return {
    name: c?.name ?? '',
    title: c?.title ?? '',
    body: c?.body ?? '',
    labels: c?.labels ?? [],
    place: c?.place ?? 'home',
    host: c?.host || null,
    agent: c?.agent || null,
    spec: spec ?? DEFAULT_SCHEDULE,
    raw: c && !spec ? normalizeCron(c.cron) : null,
  };
};

export function CardCronSection({ path, i }: { path: string; i: number }) {
  const list = useData(s => boardOf(s.ws, path)?.cardCrons ?? NONE);
  // 编辑哪一条（new 是新建）；保存失败时带着草稿重新打开
  const [open, setOpen] = useState<{ id: number | 'new'; draft?: Draft } | null>(null);
  return (
    <section className="d-section cc" style={{ ['--i' as string]: i }}>
      <div className="d-label">
        定时建卡 <span className="count">{list.length || ''}</span>
        <InfoTip text="到点按模板建一张卡，由定时领取、人派发去做，或者直接派给一台机器。上一次建的卡还没进终点、也没删时，到点不建（记「跳过」）；服务端停机期间错过的不补建" />
      </div>
      {list.length > 0 && (
        <div className="cc-list">
          {list.map(c =>
            open?.id === c.id ? (
              <CronForm key={c.id} path={path} cron={c} initial={open.draft} close={() => setOpen(null)} reopen={d => setOpen({ id: c.id, draft: d })} />
            ) : (
              <CronRow key={c.id} path={path} c={c} edit={() => setOpen({ id: c.id })} />
            ),
          )}
        </div>
      )}
      {open?.id === 'new' ? (
        <CronForm path={path} initial={open.draft} close={() => setOpen(null)} reopen={d => setOpen({ id: 'new', draft: d })} />
      ) : (
        <span data-tip={list.length >= MAX ? `每块白板最多 ${MAX} 条` : undefined}>
          <button className="btn sm cc-add" type="button" disabled={list.length >= MAX} onClick={() => setOpen({ id: 'new' })}>
            <Icon name="plus" />
            加一条定时建卡
          </button>
        </span>
      )}
    </section>
  );
}

/** 一条：开关、名称、时间、放到哪、标签；下次时间和最近一次的结果；现在建一张、编辑、删除 */
function CronRow({ path, c, edit }: { path: string; c: CardCron; edit: () => void }) {
  const cur = useCurrent();
  const labels = cur?.project.labels ?? NO_LABELS;
  const home = boardFlow(useData(s => (s.board?.path === path ? s.board : null))).home?.name ?? '待办';
  const busy = useBusy(cardCronKey(path, c.id));
  const [running, setRunning] = useState(false);
  const [removing, setRemoving] = useState(false);
  const { sync, track } = useSync();
  useNow(30_000);
  const toggle = () => {
    const on = !c.on;
    track('on', saveSetting(`cardCron:on:${path}:${c.id}`, cardCronPatch(path, c.id, { on }), () => api.updateCardCron(path, c.id, { on })));
  };
  const run = async () => {
    setRunning(true);
    try {
      const r = await optimistic(null, () => api.runCardCron(path, c.id));
      if (r.ranCard != null) {
        const id = r.ranCard;
        const open = { label: '打开', fn: () => openFromSettings(id) };
        if (r.ranErr) toast({ type: 'error', title: `「${c.name}」建了 #${id}，但没派发出去`, text: `${r.ranErr}。卡片留在「${home}」，评论里写了原因`, action: open });
        else if (r.ranTo) toast({ type: 'success', title: `「${c.name}」建了 #${id}`, text: `已派发到 ${r.ranTo}`, action: open });
        else toast({ type: 'success', title: `「${c.name}」建了 #${id}`, text: `放在${c.place === 'inbox' ? '未整理' : `「${home}」`}最下面`, action: open });
      } else if (r.ranSkip != null) toast({ type: 'info', title: `跳过：上一张 #${r.ranSkip} 还没完成`, text: '它进了终点状态或被删除后才会建下一张' });
    } catch (e) {
      toastError(e);
    } finally {
      setRunning(false);
    }
  };
  const remove = async () => {
    const yes = await confirm({ title: `删除定时建卡「${c.name}」？`, text: '之后到点不再建卡；已经建出来的卡不受影响。', ok: '删除' });
    if (!yes) return;
    setRemoving(true);
    try {
      await optimistic(null, () => api.deleteCardCron(path, c.id));
    } catch (e) {
      toastError(e);
      setRemoving(false);
    }
  };
  const tags = c.labels.map(id => labels.find(l => l.id === id)).filter(Boolean) as Label[];
  const describe = describeCron(c.cron);
  const to = useData(s => (c.host ? hostsOf(s.ws).find(h => h.name === c.host) : undefined));
  return (
    <div className={cx('cc-row', !c.on && 'off', (busy || removing) && 'saving')}>
      <button type="button" className={cx('switch', c.on && 'on')} role="switch" aria-checked={c.on} aria-label={`「${c.name}」到点建卡`} onClick={toggle}>
        <i />
      </button>
      <div className="cc-main">
        <div className="cc-l1">
          <b>{c.name}</b>
          <Sync s={sync.on} inline />
          <span className="cc-when" data-tip={`cron：${normalizeCron(c.cron)}`}>
            {describe === normalizeCron(c.cron) ? <code>{describe}</code> : describe}
            <em>{c.tz}</em>
          </span>
          <span className="cc-place">→ {c.place === 'inbox' ? '未整理' : home}</span>
          {c.host && (
            <span className={cx('cc-to', !to && 'gone')} data-tip={to ? '建出卡后马上派发到这台机器' : '这台机器已经移除：到点照样建卡，但派发不出去'}>
              <Icon name="dispatch" />
              {c.host}
              {to ? ` · ${agentName(to, c.agent)}` : '（已移除）'}
            </span>
          )}
          {tags.map(l => (
            <LabelChip key={l.id} label={l} />
          ))}
        </div>
        <div className="cc-l2">
          <span>{!c.on ? '关着：到点不建' : c.nextAt ? `下次：${when(c.nextAt)}` : '一年内不会到点'}</span>
          {c.ranAt != null && (
            <span className={cx('cc-last', (c.ranSkip != null || c.ranErr) && 'skip')}>
              {when(c.ranAt)}{' '}
              {c.ranCard != null ? (
                <>
                  建了{' '}
                  <button type="button" className="d-ref" onClick={() => openFromSettings(c.ranCard!)}>
                    #{c.ranCard}
                  </button>
                  {c.ranTo && <> · 派发到 {c.ranTo}</>}
                  {c.ranErr && <> · 没派发出去</>}
                </>
              ) : (
                <>跳过：上一张 #{c.ranSkip} 还没完成</>
              )}
            </span>
          )}
          {c.ranAt != null && c.ranCard != null && c.ranErr && <span className="cc-err">{c.ranErr}</span>}
        </div>
      </div>
      <span className="cc-acts">
        <button className={cx('btn sm', running && 'is-busy')} type="button" disabled={running || busy || removing} aria-busy={running || undefined} onClick={() => void run()}>
          现在建一张
        </button>
        <button className="icon-btn sm" type="button" aria-label={`编辑「${c.name}」`} data-tip="编辑" disabled={busy || removing} onClick={edit}>
          <Icon name="edit" />
        </button>
        <button className={cx('icon-btn sm cc-del', removing && 'is-busy')} type="button" aria-label={`删除「${c.name}」`} data-tip="删除" disabled={busy || removing} onClick={() => void remove()}>
          <Icon name="trash" />
        </button>
      </span>
    </div>
  );
}

/** 这条用的 agent 的名称：没指定时是那台机器的默认 agent */
function agentName(h: HostInfo, agent: string | null | undefined) {
  if (agent) return h.agents.find(a => a.id === agent)?.name ?? agent;
  const d = defaultAgent(h);
  return d ? `${d.name}（默认）` : '默认 agent';
}

/** 关掉白板设置，打开这张卡 */
function openFromSettings(id: number) {
  openBoardSettings(null);
  setTimeout(() => openCard(id), 120);
}

/** 新建或编辑一条：名称、卡片标题、描述、标签、放到哪、时间 */
function CronForm({ path, cron, initial, close, reopen }: { path: string; cron?: CardCron; initial?: Draft; close: () => void; reopen: (d: Draft) => void }) {
  const cur = useCurrent();
  const labels = cur?.project.labels ?? NO_LABELS;
  const home = boardFlow(useData(s => (s.board?.path === path ? s.board : null))).home?.name ?? '待办';
  const [d, setD] = useState<Draft>(() => initial ?? draftOf(cron));
  const [mode, setMode] = useState<'edit' | 'preview'>('edit');
  const [sending, setSending] = useState(false);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const setBody = (v: SetStateAction<string>) => setD(x => ({ ...x, body: typeof v === 'function' ? v(x.body) : v }));
  const attach = useAttach({ board: path, card: null, setValue: setBody, textarea: bodyRef });
  const set = (p: Partial<Draft>) => setD(x => ({ ...x, ...p }));
  const expr = d.raw ?? buildCron(d.spec);
  const ok = !!d.name.trim() && !!d.title.trim() && !!expr;
  const tz = browserTz();

  const save = async () => {
    if (!ok || stillUploading(attach.busy)) return;
    const input: CardCronInput = { name: d.name.trim(), title: d.title.trim(), body: d.body.trim(), labels: d.labels, place: d.place, host: d.host ?? '', agent: d.agent ?? '', cron: expr!, tz };
    if (!cron) {
      // 新建要等服务端返回（编号、下次时间）：按钮转圈，失败时表单留着
      setSending(true);
      try {
        await optimistic(null, () => api.createCardCron(path, input));
        close();
      } catch (e) {
        toastError(e);
      } finally {
        setSending(false);
      }
      return;
    }
    // 修改：当帧生效、收起表单；被拒绝时撤回，带着草稿重新打开
    close();
    try {
      await optimistic({ ...cardCronPatch(path, cron.id, input), keys: [cardCronKey(path, cron.id)] }, () => api.updateCardCron(path, cron.id, input));
    } catch (e) {
      toastError(e);
      reopen(d);
    }
  };

  const picked = d.labels.map(id => labels.find(l => l.id === id)).filter(Boolean) as Label[];
  return (
    <div className="cc-form" role="group" aria-label={cron ? `编辑「${cron.name}」` : '新的定时建卡'}>
      <label className="bs-rc-row">
        <span className="bs-rc-l">名称</span>
        <input className="input" value={d.name} maxLength={24} placeholder="例如：清理死代码" autoFocus={!cron} onChange={e => set({ name: e.target.value })} />
      </label>
      <label className="bs-rc-row">
        <span className="bs-rc-l">卡片标题</span>
        <span className="cc-title">
          <input className="input" value={d.title} placeholder="例如：清理死代码" onChange={e => set({ title: e.target.value })} />
          <span className="cc-hint">建卡时后面加日期，例如「{d.title.trim() || '清理死代码'}（{pad(new Date().getMonth() + 1)}-{pad(new Date().getDate())}）」</span>
        </span>
      </label>
      <div className="bs-rc-row top">
        <span className="bs-rc-l">卡片描述</span>
        <div className="cc-desc">
          <div className="cc-desc-bar">
            <button type="button" className="btn sm ghost" onClick={() => (setMode('edit'), attach.pick())}>
              <Icon name="clip" />
              {attach.busy ? '上传中…' : '附件'}
            </button>
            {attach.fileInput}
            <DescTabs
              mode={mode}
              setMode={m => setMode(m as 'edit' | 'preview')}
              modes={[
                ['edit', '编辑'],
                ['preview', '预览'],
              ]}
            />
          </div>
          <div className={cx('cc-desc-box', attach.over && 'drop-on')}>
            <textarea
              ref={bodyRef}
              {...attach.handlers}
              className="textarea"
              spellCheck={false}
              hidden={mode !== 'edit'}
              placeholder={'要做什么、发现的问题怎么处理……\n\n例如：\n找出没有调用方的函数、没人用的文件，删掉；拿不准的记进未整理。'}
              value={d.body}
              onChange={e => set({ body: e.target.value })}
            />
            {mode === 'preview' && <div className="cc-preview">{d.body.trim() ? <Markdown src={d.body} links={false} /> : <p className="cr-empty">还没有描述</p>}</div>}
          </div>
        </div>
      </div>
      <div className="bs-rc-row">
        <span className="bs-rc-l">标签</span>
        <span className="cc-labels">
          {picked.map(l => (
            <LabelChip key={l.id} label={l} onRemove={() => set({ labels: d.labels.filter(x => x !== l.id) })} />
          ))}
          <button
            type="button"
            className="lb-add"
            data-label-picker
            onClick={e =>
              cur &&
              openPicker({
                rect: e.currentTarget.getBoundingClientRect(),
                project: cur.project.key,
                selected: d.labels,
                onToggle: (id, on) => setD(x => ({ ...x, labels: on ? [...new Set([...x.labels, id])] : x.labels.filter(y => y !== id) })),
              })
            }
          >
            <Icon name="tag" />
            {picked.length ? '' : '添加标签'}
          </button>
        </span>
      </div>
      <div className="bs-rc-row">
        <span className="bs-rc-l">放到哪</span>
        <span>
          <Segmented
            value={d.place}
            options={[
              ['home', home],
              ['inbox', '未整理'],
            ]}
            onChange={v => set(v === 'inbox' ? { place: v, host: null, agent: null } : { place: v })}
          />
        </span>
      </div>
      <div className="bs-rc-row top">
        <span className="bs-rc-l">派给机器</span>
        <HostPick path={path} home={home} inbox={d.place === 'inbox'} host={d.host} agent={d.agent} onChange={(host, agent) => set({ host, agent })} />
      </div>
      <div className="bs-rc-row top">
        <span className="bs-rc-l">时间</span>
        <span className="cc-time">
          {d.raw ? (
            <span className="cc-raw">
              <code>{d.raw}</code>
              <button type="button" className="hp-link" onClick={() => set({ raw: null })}>
                重新选
              </button>
            </span>
          ) : (
            <SchedulePick spec={d.spec} onChange={spec => set({ spec })} minutes={MINUTES} />
          )}
          <span className="cc-hint">
            {expr ? `${describeCron(expr)}，` : '至少选一天，'}按 {tz}（这台电脑的时区）
          </span>
        </span>
      </div>
      <div className="cc-form-acts">
        <button className="btn sm" type="button" onClick={close}>
          取消
        </button>
        <span data-tip={!d.name.trim() ? '写个名称' : !d.title.trim() ? '写个卡片标题' : !expr ? '至少选一天' : undefined}>
          <button className={cx('btn sm primary', sending && 'is-busy')} type="button" disabled={!ok || sending} aria-busy={sending || undefined} onClick={() => void save()}>
            {cron ? '保存' : '加上'}
          </button>
        </span>
      </div>
    </div>
  );
}

/** 派给哪台机器、用哪个 agent：「不派发」或有这块白板目录的机器；选了机器再选 agent（默认是那台机器的默认 agent）。
 *  机器离线、暂停接单也能选：到点时派发排队等它。已经移除的机器还选着时照样列出来，标「已移除」 */
function HostPick({ path, home, inbox, host, agent, onChange }: { path: string; home: string; inbox: boolean; host: string | null; agent: string | null; onChange: (host: string | null, agent: string | null) => void }) {
  const ws = useData(s => s.ws);
  const hosts = hostsOf(ws);
  const h = host ? hosts.find(x => x.name === host) : undefined;
  const hasDir = (x: HostInfo) => x.dirs.some(d => d.board === path);
  const def = h ? defaultAgent(h) : undefined;
  return (
    <span className="cc-pick">
      <span className="cc-hosts" role="radiogroup" aria-label="派给机器">
        <button type="button" role="radio" aria-checked={!host} className={cx('cc-host', !host && 'on')} onClick={() => onChange(null, null)}>
          不派发
        </button>
        {hosts.map(x => {
          const ok = hasDir(x);
          const why = !x.online ? '离线' : x.paused ? '暂停接单' : '';
          return (
            <button
              key={x.id}
              type="button"
              role="radio"
              aria-checked={host === x.name}
              className={cx('cc-host', host === x.name && 'on')}
              disabled={inbox || !ok}
              data-tip={inbox ? undefined : !ok ? '没有这块白板的目录：先在机器详情的「白板与目录」里加上' : why ? `${why}：到点时派发排队，等它${!x.online ? '上线' : '恢复接单'}` : undefined}
              onClick={() => onChange(x.name, null)}
            >
              <Dot h={x} running={hostLoad(ws, x).running.length} />
              {x.name}
              {why && <em>{why}</em>}
            </button>
          );
        })}
        {host && !h && (
          <button type="button" role="radio" aria-checked className="cc-host on gone" data-tip="这台机器已经移除：到点照样建卡，但派发不出去">
            {host}
            <em>已移除</em>
          </button>
        )}
      </span>
      {h && (
        <span className="hp-agents" role="radiogroup" aria-label="agent">
          <button type="button" role="radio" aria-checked={!agent} className={cx('cc-host', !agent && 'on')} onClick={() => onChange(host, null)}>
            默认 agent{def ? `（${def.name}）` : ''}
          </button>
          {h.agents.map(a => (
            <button
              key={a.id}
              type="button"
              role="radio"
              aria-checked={agent === a.id}
              aria-label={a.name}
              className={cx('hp-agent-btn', agent === a.id && 'on')}
              disabled={!a.found && agent !== a.id}
              data-tip={a.found ? undefined : a.env ? `在${envName(h, a.env)}里没找到命令` : '这台机器上没找到命令'}
              onClick={() => onChange(host, a.id)}
            >
              {/* 默认的那个写在「默认 agent（…）」里，chip 上不再标，免得看起来像选中了两个 */}
              <AgentChip a={a} env={envName(h, a.env)} />
            </button>
          ))}
        </span>
      )}
      <span className="cc-hint">
        {inbox
          ? '放到未整理的不派发：要派给机器，「放到哪」选「' + home + '」'
          : host
            ? `到点建卡后马上派发到 ${host}（和手动派发一样：机器离线、暂停接单或没有空位时排队）；派不出去时卡留在「${home}」，评论写明原因`
            : hosts.length
              ? '只建卡，由定时领取或人派发去做'
              : '还没有机器：在要干活的机器上运行 board host，它会出现在这里'}
      </span>
    </span>
  );
}
