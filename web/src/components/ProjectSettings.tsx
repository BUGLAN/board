/* ==========================================================================
   项目设置（PRD §10.4「项目设置」）：项目菜单里的「项目设置」、白板设置末尾的「打开项目设置」打开
   和白板设置同一种居中弹窗（.scrim / .drawer / .d-*，手机上是底部 sheet），单栏：
   - 基本信息：名称（必填）、标识（只读）、描述、颜色（和新建项目同一组颜色）
   - 代码仓库：仓库地址、集成分支（一个项目一个仓库，项目下所有白板共用）；没设时一行警告，有机器报过 origin 时可以一键用它
   - 标签：项目的标签，「管理标签」叠在上面打开
   每一项都是设置项（saveSetting）：失焦或回车保存，Esc 取消这一次输入，保存状态画在那一项旁边，不整块禁用；名称清空时回到原值
   ========================================================================== */
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { api } from '../lib/api';
import { PROJECT_COLORS } from '../lib/schema';
import type { ProjectInfo } from '../lib/types';
import { cx } from '../lib/util';
import { patchProject, repoPatch, saveSetting, useData } from '../store/data';
import { manageLabels, openProjectSettings, useUI } from '../store/ui';
import { InfoTip, Sync, useSync } from './bits';
import { Icon } from './Icon';

export function ProjectSettings() {
  const pk = useUI(s => s.projectSettings);
  const on = pk != null;
  // 关的时候弹窗淡出，内容（最后打开的那个项目）留到动画结束
  const [last, setLast] = useState(pk);
  if (pk && pk !== last) setLast(pk);
  const p = useData(s => s.ws?.projects.find(x => x.key === (pk ?? last)) ?? null);
  const [shown, setShown] = useState(on);
  useEffect(() => {
    if (on) return setShown(true);
    const t = setTimeout(() => setShown(false), 420);
    return () => clearTimeout(t);
  }, [on]);
  const close = () => openProjectSettings(null);
  useEffect(() => {
    if (!on) return;
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      // 叠在上面的（管理标签、确认框、选择框、标签选择器、命令面板）先关自己
      const ui = useUI.getState();
      if (ui.ctx || ui.confirm || ui.palette.open || ui.labelsOf || ui.picker) return;
      // 正在输入：这一次 Esc 只取消输入（输入框自己处理），再按一次才关弹窗
      if ((e.target as HTMLElement | null)?.closest?.('.ps-modal input:not([readonly])')) return;
      e.preventDefault();
      e.stopPropagation();
      close();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [on]);
  // 项目被删了：弹窗跟着关
  useEffect(() => {
    if (on && !p) close();
  }, [on, p]); // eslint-disable-line react-hooks/exhaustive-deps
  return createPortal(
    <>
      <div className={cx('scrim', on && 'on')} onClick={close} />
      <aside className={cx('drawer bs-modal ps-modal', on && 'on')} role="dialog" aria-modal="true" aria-hidden={!on} aria-label="项目设置">
        {p && shown && <Body key={p.key} p={p} close={close} />}
      </aside>
    </>,
    document.body,
  );
}

/** 一行输入：失焦或回车保存，Esc 取消这一次输入；没在编辑时跟着服务端（别的窗口改了） */
function Field({
  label, value, placeholder, mono, required, readOnly, hint, extra, saving, onSave,
}: {
  label: string;
  value: string;
  placeholder?: string;
  mono?: boolean;
  required?: boolean;
  readOnly?: boolean;
  hint?: string;
  extra?: ReactNode;
  /** 外面（「用机器的 origin」）发起的保存：状态也画在这一项旁边 */
  saving?: ReturnType<typeof useSync>;
  onSave?: (v: string) => Promise<unknown>;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);
  // Esc 之后的失焦不保存（失焦时 draft 还是这一次输入的值）
  const cancelled = useRef(false);
  const own = useSync();
  const { sync, track } = saving ?? own;
  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value]); // eslint-disable-line react-hooks/exhaustive-deps
  const commit = () => {
    setEditing(false);
    if (cancelled.current) {
      cancelled.current = false;
      return setDraft(value);
    }
    const v = draft.trim();
    if (v === value || (required && !v)) return setDraft(value);
    setDraft(v);
    if (onSave) track('f', onSave(v));
  };
  return (
    <label className="bs-rc-row">
      <span className="bs-rc-l">
        {label}
        <Sync s={sync.f} inline />
      </span>
      <span className="bs-rp-input">
        <input
          className={cx('input', mono && 'mono')}
          value={draft}
          placeholder={placeholder}
          readOnly={readOnly}
          title={hint}
          spellCheck={false}
          autoComplete="off"
          aria-label={label}
          onFocus={() => !readOnly && setEditing(true)}
          onChange={e => setDraft(e.target.value)}
          onBlur={() => !readOnly && commit()}
          onKeyDown={e => {
            if (readOnly) return;
            if (e.key === 'Enter') e.currentTarget.blur();
            if (e.key === 'Escape' && editing) {
              // 只取消这一次输入，不关弹窗
              e.stopPropagation();
              cancelled.current = true;
              e.currentTarget.blur();
            }
          }}
        />
        {extra}
      </span>
    </label>
  );
}

function Body({ p, close }: { p: ProjectInfo; close: () => void }) {
  const ws = useData(s => s.ws);
  const color = useSync();
  const repoSync = useSync();
  const pk = p.key;
  const repo = p.repo ?? '';
  const saveProject = (field: 'name' | 'desc' | 'color', v: string) =>
    saveSetting(`proj:${pk}:${field}`, patchProject(pk, x => ({ ...x, [field]: v })), () => api.updateProject(pk, { [field]: v }));
  const saveRepo = (which: 'repo' | 'branch', v: string) =>
    saveSetting(`${which}:${pk}`, repoPatch(pk, { [which]: v }), () => api.updateProject(pk, which === 'repo' ? { repo: v } : { branch: v }));
  // 机器上报的这个项目目录的 origin（#185 克隆也用同一个来源）：仓库地址没设时作建议
  const suggested = ws?.hosts?.flatMap(h => h.dirs ?? []).find(d => d.board.split('/')[0] === pk && d.origin)?.origin ?? '';

  return (
    <>
      <header className="d-head">
        <span className="bs-ttl">
          <Icon name="folder" />
          项目设置
        </span>
        <span className="d-col bs-where">
          <i style={{ background: p.color }} />
          {p.name}
        </span>
        <span className="ps-sub">
          {p.boards.length} 块白板：{p.boards.map(b => b.name).join('、')}
        </span>
        <span className="spacer" />
        <button type="button" className="icon-btn sm" aria-label="关闭" title="关闭（Esc）" onClick={close}>
          <Icon name="close" />
        </button>
      </header>
      <div className="drawer-body">
        <div className="d-grid ps-grid">
          <div className="d-main ps-main">
            <section className="d-section" style={{ ['--i' as string]: 0 }}>
              <div className="d-label">基本信息</div>
              <div className="bs-rc bs-rp">
                <Field label="名称" value={p.name} required onSave={v => saveProject('name', v)} />
                <Field label="标识" value={p.key} mono readOnly hint="创建后不能修改" />
                <Field label="描述" value={p.desc ?? ''} placeholder="一句话说明这个项目（选填）" onSave={v => saveProject('desc', v)} />
                <div className="bs-rc-row">
                  <span className="bs-rc-l">
                    颜色
                    <Sync s={color.sync.c} inline />
                  </span>
                  <span className="ps-colors" role="radiogroup" aria-label="颜色">
                    {PROJECT_COLORS.map(c => (
                      <button
                        key={c}
                        type="button"
                        role="radio"
                        className={cx('ps-color', p.color.toUpperCase() === c.toUpperCase() && 'on')}
                        style={{ background: c }}
                        aria-label={`颜色 ${c}`}
                        aria-checked={p.color.toUpperCase() === c.toUpperCase()}
                        onClick={() => p.color.toUpperCase() !== c.toUpperCase() && color.track('c', saveProject('color', c))}
                      />
                    ))}
                  </span>
                </div>
              </div>
            </section>

            <section className="d-section" style={{ ['--i' as string]: 1 }}>
              <div className="d-label">
                代码仓库
                <InfoTip text="一个项目一个仓库，项目下所有白板共用，机器上的目录是 <工作空间>/<项目>。设了仓库地址后，描述和评论里的「提交：abc1234」和 gh#12 变成链接，机器上还没有这个项目的目录时按它克隆。集成分支是 agent 推送的目标，留空按仓库的 AGENTS.md / CLAUDE.md，没写用远端默认分支" />
              </div>
              <div className="bs-rc bs-rp">
                <Field
                  label="仓库地址"
                  value={repo}
                  mono
                  placeholder={suggested ? `例如 ${suggested}` : 'git@github.com:org/repo.git'}
                  saving={repoSync}
                  onSave={v => saveRepo('repo', v)}
                  extra={
                    !repo.trim() && suggested ? (
                      <button type="button" className="btn sm" title={`用机器上报的 origin：${suggested}`} onClick={() => repoSync.track('f', saveRepo('repo', suggested))}>
                        用机器的 origin
                      </button>
                    ) : null
                  }
                />
                <Field label="集成分支" value={p.branch ?? ''} mono placeholder="留空按 AGENTS.md / CLAUDE.md，没写用远端默认分支" onSave={v => saveRepo('branch', v)} />
              </div>
              {!repo.trim() && (
                <p className="ps-note warn" role="status">
                  <Icon name="alert" />
                  还没设仓库地址：机器上这个项目的目录会是空目录，agent 没有代码可改、也推不了。
                </p>
              )}
            </section>

            <section className="d-section" style={{ ['--i' as string]: 2 }}>
              <div className="d-label">
                标签
                <span className="act">{p.labels.length} 个 · 项目下所有白板共用</span>
              </div>
              <div className="ps-labels">
                {p.labels.map(l => (
                  <span key={l.id} className="ps-label" style={{ ['--c' as string]: l.color }}>
                    {l.name}
                  </span>
                ))}
                {p.labels.length === 0 && <span className="ps-none">还没有标签</span>}
                <button type="button" className="btn sm" onClick={() => manageLabels(pk)}>
                  <Icon name="tag" />
                  管理标签
                </button>
              </div>
            </section>
            <div className="bs-foot">改动立即生效，同步到所有窗口和机器。新建、删除白板在「项目」tab 里。</div>
          </div>
        </div>
      </div>
    </>
  );
}
