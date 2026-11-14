/* ==========================================================================
   白板的环境检查（PRD §9.1「环境检查」，白板设置里的一节）
   - 检查命令：所有系统一条，Windows 可以单独写一条（cmd 里执行）。设置项（saveSetting）：失焦或回车时保存，
     不锁别的控件，保存状态画在输入框旁边
   - 能接这块白板的机器：配了工作空间的每台机器一行（有工作空间就视为有这块白板的目录；在线、检查结果、
     上次检查时间、那块白板派生的目录），点一行打开机器详情。一台都没有时写提醒，并给出去机器「工作空间」的入口；
     大盘总览里同样的提醒用 NoHostsNote
   ========================================================================== */
import { useEffect, useState } from 'react';
import { api } from '../lib/api';
import { boardHosts, boardOf, hostLoad, hostsOf, osKind, since, type Readiness } from '../lib/hosts';
import { cx } from '../lib/util';
import { readyCheckPatch, saveSetting, useData } from '../store/data';
import { openBoardSettings, openHost } from '../store/ui';
import { InfoTip, Sync, useNow, useSync } from './bits';
import { Dot } from './hosts/bits';
import { Icon } from './Icon';

const LABEL: Record<Readiness['key'], string> = { none: '不检查', ok: '通过', failed: '没通过', pending: '还没检查' };

/** 去机器的「工作空间」：有机器就打开第一台的详情，没有就打开「添加机器」 */
function goDirs() {
  const first = hostsOf(useData.getState().ws)[0];
  openBoardSettings(null);
  openHost(first ? first.name : 'new');
}

export function ReadyCheckSection({ path, i }: { path: string; i: number }) {
  const ws = useData(s => s.ws);
  const rc = boardOf(ws, path)?.readyCheck;
  const rows = boardHosts(ws, path);
  const now = useNow(30_000);
  const { sync, track } = useSync();
  const [draft, setDraft] = useState({ cmd: rc?.cmd ?? '', win: rc?.win ?? '' });
  const [editing, setEditing] = useState<'' | 'cmd' | 'win'>('');
  // 别的窗口改了：没在编辑的那一项跟着服务端
  useEffect(() => {
    setDraft(d => ({ cmd: editing === 'cmd' ? d.cmd : rc?.cmd ?? '', win: editing === 'win' ? d.win : rc?.win ?? '' }));
  }, [rc?.cmd, rc?.win]); // eslint-disable-line react-hooks/exhaustive-deps

  const commit = (which: 'cmd' | 'win') => {
    setEditing('');
    const v = draft[which].trim();
    if (v === (rc?.[which] ?? '')) return;
    setDraft(d => ({ ...d, [which]: v }));
    track(
      which,
      saveSetting(`ready:${path}:${which}`, readyCheckPatch(path, { [which]: v }), () =>
        api.updateBoard(path, which === 'cmd' ? { readyCheck: v } : { readyCheckWin: v }),
      ),
    );
  };
  const field = (which: 'cmd' | 'win', label: string, hint: string) => (
    <label className="bs-rc-row">
      <span className="bs-rc-l">
        {label}
        <Sync s={sync[which]} inline />
      </span>
      <input
        className="input mono"
        value={draft[which]}
        placeholder={hint}
        spellCheck={false}
        autoComplete="off"
        aria-label={`环境检查：${label}`}
        onFocus={() => setEditing(which)}
        onChange={e => setDraft(d => ({ ...d, [which]: e.target.value }))}
        onBlur={() => commit(which)}
        onKeyDown={e => {
          if (e.key === 'Enter') e.currentTarget.blur();
          if (e.key === 'Escape' && editing) {
            e.stopPropagation();
            setDraft(d => ({ ...d, [which]: rc?.[which] ?? '' }));
            setEditing('');
            e.currentTarget.blur();
          }
        }}
      />
    </label>
  );
  const on = !!(rc?.cmd.trim() || rc?.win.trim());
  return (
    <section className="d-section" style={{ ['--i' as string]: i }}>
      <div className="d-label">
        环境检查
        <InfoTip text="没通过的机器不接这块白板的卡。在每台机器上这块白板的目录里执行（macOS、Linux 在登录 shell 里，Windows 在 cmd 里），退出码 0 算通过，最长 2 分钟；board host 启动、重新读配置、每天一次、命令改了之后各跑一次。留空不检查" />
      </div>
      <div className="bs-rc">
        {field('cmd', '所有系统', '例如 cargo --version && node --version')}
        {field('win', 'Windows', '留空和上面一样；写了就用它（cmd 里执行）')}
      </div>

      <div className="d-label bs-rc-hosts">
        能接这块白板的机器<span className="act">{rows.length ? `${rows.filter(r => r.ready.key === 'none' || r.ready.key === 'ok').length}/${rows.length} 台能接` : ''}</span>
      </div>
      {rows.length === 0 ? (
        <div className="bs-rc-empty" role="status">
          <Icon name="alert" />
          <span>
            还没有机器能接这块白板的卡：定时领取不会领它的卡，也派发不了。给机器配上工作空间（白板的目录是 &lt;工作空间&gt;/{path}）就能接。
          </span>
          <button type="button" className="btn sm" onClick={goDirs}>
            去设置工作空间
          </button>
        </div>
      ) : (
        <div className="bs-rc-list">
          {rows.map(({ h, d, ready }) => (
            <button key={h.id} type="button" className={cx('bs-rc-host', ready.key)} onClick={() => openHost(h.name)} aria-label={`打开机器 ${h.name}`}>
              <Dot h={h} running={hostLoad(ws, h).running.length} />
              <b>{h.name}</b>
              <span className="bs-rc-on">{h.online ? '在线' : '离线'}</span>
              <span className="bs-rc-os">{osKind(h.os) === 'win' ? 'Windows' : osKind(h.os) === 'mac' ? 'macOS' : 'Linux'}</span>
              <span className={cx('bs-rc-st', ready.key)}>{on ? LABEL[ready.key] : LABEL.none}</span>
              <span className="bs-rc-at">{'at' in ready ? `检查于 ${since(ready.at, now)}` : ''}</span>
              <span className="bs-rc-dir mono" title={d.path}>
                {d.path}
              </span>
              {ready.key === 'failed' && <span className="bs-rc-why mono">{ready.last}</span>}
              {ready.key === 'pending' && <span className="bs-rc-why">等 board host 跑完；旧版 board host 不会检查，先升级 board</span>}
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

/** 大盘总览：一台机器都没有配工作空间（接得了这块白板）时的提醒 */
export function NoHostsNote({ path }: { path: string }) {
  const none = useData(s => boardHosts(s.ws, path).length === 0);
  if (!none) return null;
  return (
    <div className="bs-rc-empty sp-ov-nohost" role="status">
      <Icon name="alert" />
      <span>还没有机器能接这块白板的卡：定时领取不会领它的卡，也派发不了。给机器配上工作空间就能接。</span>
      <button type="button" className="btn sm" onClick={goDirs}>
        去设置工作空间
      </button>
    </div>
  );
}
