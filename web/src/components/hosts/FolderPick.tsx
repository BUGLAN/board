/* ==========================================================================
   给一台机器选工作空间的文件夹：盖在机器详情上的分栏视图（像访达）
   - 服务端给那台机器的 board host 发「列目录」请求，它回传子文件夹（名称、是不是 git 仓库、里面有几个 git 仓库）
   - 一层一列，打开时展开到现在的工作空间；点一个文件夹右边多一列它的子文件夹，正在取的那一列画骨架
   - git 仓库本身置灰、不能点进去（工作空间是放仓库的那一层）；现在的工作空间画一个圆点
   - 底部：选中的路径和一句说明，「取消」「用这个文件夹」（选的就是现在的工作空间时写「现在就是它」、不能点）
   - 列不出来（离线、旧版 board host、没回话）：写原因，可以直接填路径
   本机在桌面端里时不用它，弹系统的文件夹选择框（HostSettings.tsx）
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../../lib/api';
import type { DirList, HostInfo } from '../../lib/types';
import { cx, isImeKey } from '../../lib/util';
import { Icon } from '../Icon';

const isWin = (h: HostInfo) => /windows/i.test(h.os);
const label = (h: HostInfo, p: string) => (p === '' ? (isWin(h) ? '此电脑' : '常用') : p);
type Col = DirList | { error: string } | undefined;

export function FolderPick({ h, current, onPick, onClose }: { h: HostInfo; current: string; onPick: (p: string) => void; onClose: () => void }) {
  /** 从最上面一层到选中的那一层 */
  const [chain, setChain] = useState<string[] | null>(null);
  const [lists, setLists] = useState<Record<string, Col>>({});
  /** 现在的工作空间（~ 展开后的） */
  const [now, setNow] = useState<string | null>(null);
  const [fail, setFail] = useState<string | null>(null);
  const [typed, setTyped] = useState(current);
  const asked = useRef(new Set<string>());
  const cols = useRef<HTMLDivElement>(null);

  const ask = (p: string) => {
    if (asked.current.has(p)) return;
    asked.current.add(p);
    api.hostLs(h.name, p).then(
      x => setLists(l => ({ ...l, [p]: x })),
      e => setLists(l => ({ ...l, [p]: { error: String((e as Error)?.message ?? e) } })),
    );
  };

  // 打开时：先问现在的工作空间（顺带拿到它的每一级），再把上面几级也问一遍
  useEffect(() => {
    let alive = true;
    const start = current.trim();
    api.hostLs(h.name, start).then(
      x => {
        if (!alive) return;
        asked.current.add(start);
        const at = x.path;
        if (x.error && start) {
          // 现在的工作空间列不出来（不存在了）：从最上面一层开始
          setChain(['']);
          ask('');
          return;
        }
        setNow(start ? at : null);
        setLists(l => ({ ...l, [at]: x, ...(at !== start ? { [start]: x } : {}) }));
        asked.current.add(at);
        const c = x.chain.length ? x.chain : [at];
        setChain(c);
        c.forEach(ask);
      },
      e => alive && setFail(String((e as Error)?.message ?? e)),
    );
    return () => {
      alive = false;
    };
  }, [h.name]); // eslint-disable-line react-hooks/exhaustive-deps

  // 新开的一栏滚进来
  useLayoutEffect(() => {
    const el = cols.current;
    if (el) el.scrollTo({ left: el.scrollWidth, behavior: 'smooth' });
  }, [chain?.length]);

  // esc 只关这一层
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || isImeKey(e)) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [onClose]);

  const open = (k: number, path: string) => {
    if (!chain) return;
    setChain([...chain.slice(0, k + 1), path]);
    ask(path);
  };
  const sel = chain ? chain[chain.length - 1] : '';
  const here = lists[sel];
  const entries = here && 'entries' in here ? here.entries : undefined;
  const repos = entries?.filter(x => x.git).length ?? 0;
  const same = !!now && sel === now;

  return createPortal(
    <>
      <div className="hd-sheet-scrim" onClick={onClose} />
      <div className="hd-sheet" role="dialog" aria-label={`选择 ${h.name} 的工作空间`}>
        <header className="hd-sheet-head">
          <div>
            <b>选择工作空间</b>
            <span>{h.name} 上的文件夹 · 每个项目的目录会放在这里面</span>
          </div>
          <button type="button" className="icon-btn" aria-label="关闭" onClick={onClose}>
            <Icon name="close" />
          </button>
        </header>

        <div className="hd-cols" ref={cols}>
          {fail ? (
            <div className="hd-fcol hd-fcol-fail">
              <p className="hd-fcol-empty">
                <Icon name="alert" />
                列不出 {h.name} 的文件夹：{fail}
              </p>
            </div>
          ) : !chain ? (
            <Wait name={h.name} />
          ) : (
            chain.map((p, k) => {
              const list = lists[p];
              const next = chain[k + 1];
              if (list === undefined) return <Wait key={p || 'root'} name={h.name} />;
              if ('error' in list && !('entries' in list))
                return (
                  <div className="hd-fcol" key={p || 'root'}>
                    <p className="hd-fcol-empty">{list.error}</p>
                  </div>
                );
              const l = list as DirList;
              return (
                <div className="hd-fcol" key={p || 'root'}>
                  {l.error ? (
                    <p className="hd-fcol-empty">列不出来：{l.error}</p>
                  ) : l.entries.length === 0 ? (
                    <p className="hd-fcol-empty">没有子文件夹</p>
                  ) : (
                    l.entries.map(e => {
                      const on = next === e.path;
                      return (
                        <button
                          type="button"
                          key={e.path}
                          className={cx('hd-item', on && 'on', on && k === chain.length - 2 && 'sel', e.git && 'git')}
                          onClick={() => open(k, e.path)}
                          disabled={e.git}
                          title={e.git ? '这是一个 git 仓库：工作空间要选放仓库的那一层' : e.path}
                        >
                          <Icon name={e.git ? 'merge' : 'folder'} />
                          <span className="hd-item-name">{e.name}</span>
                          {e.path === now && <i className="hd-now" title="现在的工作空间" />}
                          {!e.git && e.repos > 0 && <small>{e.repos}</small>}
                          {!e.git && <Icon name="chevRight" />}
                        </button>
                      );
                    })
                  )}
                </div>
              );
            })
          )}
        </div>

        <footer className="hd-sheet-foot">
          {fail ? (
            <>
              <div className="hd-sheet-sel">
                <input className="input mono" value={typed} placeholder={isWin(h) ? 'D:\\repo' : '~/repo'} spellCheck={false} autoComplete="off" aria-label="工作空间路径" onChange={e => setTyped(e.target.value)} />
                <small>可以直接填路径：保存后那台机器的 board host 换上新工作空间</small>
              </div>
              <button type="button" className="btn sm ghost" onClick={onClose}>
                取消
              </button>
              <button type="button" className="btn sm brand" disabled={!typed.trim() || typed.trim() === current} onClick={() => onPick(typed.trim())}>
                {typed.trim() === current ? '现在就是它' : '用这个路径'}
              </button>
            </>
          ) : (
            <>
              <div className="hd-sheet-sel">
                <span className="hd-path">
                  <Icon name="folder" />
                  {label(h, sel)}
                </span>
                <small>
                  {!sel
                    ? '先点进一个文件夹'
                    : here === undefined
                      ? '正在看里面有什么…'
                      : repos
                        ? `里面有 ${repos} 个 git 仓库，会直接当项目目录用`
                        : '里面还没有仓库：派来卡片时按 <工作空间>/<项目> 建好或克隆'}
                </small>
              </div>
              <button type="button" className="btn sm ghost" onClick={onClose}>
                取消
              </button>
              <button type="button" className="btn sm brand" disabled={!sel || same} onClick={() => onPick(sel)}>
                {same ? '现在就是它' : '用这个文件夹'}
              </button>
            </>
          )}
        </footer>
      </div>
    </>,
    document.body,
  );
}

/** 正在问那台机器的那一栏：骨架 */
function Wait({ name }: { name: string }) {
  return (
    <div className="hd-fcol">
      <div className="hd-fcol-wait" aria-label={`正在问 ${name}`}>
        {[70, 54, 82, 46].map((w, i) => (
          <i key={i} style={{ width: `${w}%` }} />
        ))}
      </div>
    </div>
  );
}
