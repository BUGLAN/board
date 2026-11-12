/* 确认框、表单弹窗（新建 / 编辑项目和白板） */
import { useEffect, useLayoutEffect, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from 'react';
import { stillUploading, useAttach } from '../../lib/attach';
import type { ProjectInfo } from '../../lib/types';
import { autosize, cx, replay } from '../../lib/util';
import { useData } from '../../store/data';
import { closeForm, settleConfirm, useUI, type ConfirmReq, type FormCfg, type FormField } from '../../store/ui';
import { ColorRow, Markdown, ProjectAvatar, Select } from '../bits';
import { Icon, type IconName } from '../Icon';
import { usePresence } from './presence';

/** 工作区还没取到时的项目列表：模块级常量，选择器每次返回同一个数组（新数组会让 useData 认为变了、反复重渲染） */
const NO_PROJECTS: ProjectInfo[] = [];

/** 确认框的图标；不指定时按删除（危险）画 */
const ICONS: Record<NonNullable<ConfirmReq['icon']>, IconName> = {
  trash: 'trash',
  tag: 'tag',
  reset: 'renew',
  logout: 'logout',
  key: 'key',
  unlock: 'unlock',
  alert: 'alert',
  host: 'host',
  update: 'renew',
};

/** 确认框里的勾选项：勾没勾写回 box，由 confirmCheck 读出来 */
function ConfirmCheck({ check }: { check: NonNullable<ConfirmReq['check']> }) {
  const [on, setOn] = useState(check.box.checked);
  return (
    <label className={cx('dialog-check', check.why && 'off')} title={check.why}>
      <input
        type="checkbox"
        checked={on}
        disabled={!!check.why}
        onChange={e => {
          check.box.checked = e.target.checked;
          setOn(e.target.checked);
        }}
      />
      <span>
        {check.label}
        {check.why && <em>{check.why}</em>}
      </span>
    </label>
  );
}

export function ConfirmDialog() {
  const req = useUI(s => s.confirm);
  const { mounted, on } = usePresence(!!req, 220);
  const last = useRef(req);
  if (req) last.current = req;
  const r = last.current;
  const cancel = useRef<HTMLButtonElement>(null);
  const ok = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!req) return;
    const prev = document.activeElement as HTMLElement | null;
    const t = setTimeout(() => cancel.current?.focus(), 40);
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        e.preventDefault();
        settleConfirm(false);
      }
      if (e.key === 'Tab') {
        e.preventDefault();
        (document.activeElement === ok.current ? cancel.current : ok.current)?.focus();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => {
      clearTimeout(t);
      document.removeEventListener('keydown', key, true);
      prev?.focus?.({ preventScroll: true });
    };
  }, [req]);
  if (!mounted || !r) return null;
  const lines = r.text.split('\n');
  // 「发现新版本」不是危险动作：图标用品牌色、主按钮用品牌色
  const info = r.icon === 'update';
  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && settleConfirm(false)}>
      <div
        className={cx('dialog', r.tone === 'warn' && 'warn', (r.markdown || lines.length > 1 || r.text.length > 240) && 'wide')}
        role="alertdialog"
        aria-modal="true"
        aria-label={r.title}
      >
        <div className={cx('dialog-icon', info && 'info')}>
          <Icon name={ICONS[r.icon ?? 'trash'] ?? 'trash'} />
        </div>
        <h3>{r.title}</h3>
        {r.markdown ? (
          // 正文是 Markdown（更新说明来自 CHANGELOG）：限高、在框里滚动
          <div className="dialog-md md">
            <Markdown src={r.text} />
          </div>
        ) : (
          /* 按行拆成段落（目录列表一行一条）；整块限高、在框里滚动，长列表不撑破弹窗 */
          <div className="dialog-text">
            {lines.map((line, i) => (line.trim() ? <p key={i}>{line}</p> : null))}
          </div>
        )}
        {r.check && <ConfirmCheck key={r.title} check={r.check} />}
        <div className="dialog-actions">
          <button className="btn" type="button" ref={cancel} onClick={() => settleConfirm(false)}>
            {r.cancel || '取消'}
          </button>
          <button className={cx('btn', info ? 'brand' : 'danger-solid')} type="button" ref={ok} onClick={() => settleConfirm(true)}>
            {r.ok || '确定'}
          </button>
        </div>
      </div>
    </div>
  );
}

/* ============================ 表单弹窗：项目 / 白板 ============================ */

export function FormModal() {
  const cfg = useUI(s => s.form);
  const { mounted, on } = usePresence(!!cfg, 220);
  const last = useRef(cfg);
  if (cfg) last.current = cfg;
  if (!mounted || !last.current) return null;
  return <FormBody key={last.current.title + (cfg ? '1' : '0')} cfg={last.current} on={on && !!cfg} />;
}

function FormBody({ cfg, on }: { cfg: FormCfg; on: boolean }) {
  const [v, setV] = useState<Record<string, string>>(() => Object.fromEntries(cfg.fields.map(f => [f.name, f.value ?? ''])));
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [uploading, setUploading] = useState(0);
  const form = useRef<HTMLFormElement>(null);

  useEffect(() => {
    const t = setTimeout(() => {
      if (cfg.focus) {
        const el = form.current?.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[name="${cfg.focus}"]`);
        el?.focus();
        el?.setSelectionRange(el.value.length, el.value.length);
        el?.scrollIntoView({ block: 'nearest' });
      } else form.current?.querySelector<HTMLElement>('input:not([readonly]), select')?.focus();
    }, 40);
    return () => clearTimeout(t);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const update = (name: string, value: string) => {
    setV(prev => {
      const next = { ...prev, [name]: value };
      // 标识自动跟随名称，直到用户手动改过
      for (const f of cfg.fields)
        if (f.type === 'key' && f.from && !f.readonly && !touched[f.name] && f.name !== name)
          next[f.name] = next[f.from]?.trim() ? (f.auto?.(next[f.from], next) ?? '') : '';
      return next;
    });
    setErr('');
  };
  const live = cfg.fields.map(f => (f.validate && v[f.name] ? f.validate(v[f.name], v) : null)).find(Boolean) || '';

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const first = cfg.fields.find(f => f.required && !String(v[f.name] ?? '').trim());
    if (first) {
      const el = form.current?.elements.namedItem(first.name) as HTMLElement | null;
      replay(el, 'invalid');
      el?.focus();
      return;
    }
    if (live) return setErr(live);
    if (stillUploading(uploading)) return;
    setBusy(true);
    try {
      await cfg.onSubmit(v);
      closeForm();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const color = v.color;
  const pv = cfg.previewData ?? {};
  const projects = useData(s => s.ws?.projects ?? NO_PROJECTS);
  let preview: ReactNode = null;
  if (cfg.preview === 'project' || cfg.preview === 'edit-project') {
    preview = (
      <>
        <ProjectAvatar name={v.name || '新'} color={color} />
        <div>
          <b>{v.name || '新项目'}</b>
          <span className="mono">
            {cfg.preview === 'project' ? `export BOARD=${v.key || '…'}/${v.boardKey || 'main'}` : `${pv.key} · ${pv.boards} 块白板`}
          </span>
        </div>
      </>
    );
  } else if (cfg.preview === 'board' || cfg.preview === 'edit-board') {
    const p = projects.find(x => x.key === (v.project || pv.path?.split('/')[0]));
    preview = (
      <>
        <span className="pj-bic lg" style={{ ['--pc' as string]: p?.color ?? pv.color }}>
          <Icon name="board" />
        </span>
        <div>
          <b>
            {p?.name ?? pv.projectName} / {v.name || '新白板'}
          </b>
          <span className="mono">{cfg.preview === 'board' ? `export BOARD=${v.project}/${v.key || '…'}` : `BOARD=${pv.path}`}</span>
        </div>
      </>
    );
  }

  return (
    <div className={cx('dialog-wrap', on && 'on')} onClick={e => e.target === e.currentTarget && closeForm()}>
      <form
        className={cx('dialog pj-modal', cfg.fields.some(f => f.type === 'markdown') && 'wide')}
        ref={form}
        role="dialog"
        aria-modal="true"
        noValidate
        onSubmit={submit}
        onKeyDown={e => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            closeForm();
          }
        }}
      >
        <h3>{cfg.title}</h3>
        {cfg.subtitle && <p>{cfg.subtitle}</p>}
        {preview && <div className="pj-preview">{preview}</div>}
        {cfg.fields.map(f => {
          const hint = f.hint ? <em>{f.hint}</em> : null;
          if (f.type === 'color')
            return (
              <div className="field" key={f.name}>
                <span>
                  {f.label}
                  {hint}
                </span>
                <ColorRow value={v[f.name] ?? ''} onPick={c => update(f.name, c)} label={f.label} />
              </div>
            );
          if (f.type === 'markdown')
            return (
              <MdField
                key={f.name}
                f={f}
                value={v[f.name] ?? ''}
                setValue={up => {
                  setV(prev => ({ ...prev, [f.name]: typeof up === 'function' ? up(prev[f.name] ?? '') : up }));
                  setErr('');
                }}
                onBusy={setUploading}
              />
            );
          if (f.type === 'select')
            return (
              <label className="field" key={f.name}>
                <span>
                  {f.label}
                  {hint}
                </span>
                <Select name={f.name} value={v[f.name]} options={f.options ?? []} onChange={val => update(f.name, val)} />
              </label>
            );
          return (
            <label className="field" key={f.name}>
              <span>
                {f.label}
                {hint}
              </span>
              <input
                className={cx('input', f.type === 'key' && 'mono')}
                name={f.name}
                value={v[f.name] ?? ''}
                placeholder={f.placeholder}
                maxLength={f.type === 'key' ? 32 : 500}
                autoComplete="off"
                spellCheck={false}
                readOnly={f.readonly}
                onChange={e => {
                  let val = e.target.value;
                  if (f.type === 'key') {
                    val = val.toLowerCase().replace(/[^a-z0-9-]/g, '');
                    setTouched(t => ({ ...t, [f.name]: true }));
                  }
                  update(f.name, val);
                }}
              />
            </label>
          );
        })}
        {(err || live) && <div className="field-err">{err || live}</div>}
        <div className="dialog-actions">
          <button className="btn" type="button" onClick={closeForm}>
            取消
          </button>
          <button className={cx('btn brand', busy && 'is-busy')} type="submit" disabled={busy} aria-busy={busy || undefined}>
            {cfg.submit || '保存'}
          </button>
        </div>
      </form>
    </div>
  );
}

/** 多行 Markdown 字段：能粘贴截图、拖入文件（上传到 f.board 这块白板，还不属于任何卡片） */
function MdField({ f, value, setValue, onBusy }: { f: FormField; value: string; setValue: Dispatch<SetStateAction<string>>; onBusy: (n: number) => void }) {
  const ta = useRef<HTMLTextAreaElement>(null);
  const attach = useAttach({ board: f.board, card: null, setValue, textarea: ta });
  useLayoutEffect(() => autosize(ta.current), [value]);
  useEffect(() => onBusy(attach.busy), [attach.busy]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="field">
      <span>
        {f.label}
        {f.hint && <em>{f.hint}</em>}
      </span>
      <textarea
        ref={ta}
        className={cx('input md-input', attach.over && 'drop')}
        name={f.name}
        value={value}
        placeholder={f.placeholder}
        rows={4}
        spellCheck={false}
        onChange={e => setValue(e.target.value)}
        onKeyDown={e => {
          // 回车换行，不提交表单；⌘/Ctrl + 回车保存
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            e.currentTarget.form?.requestSubmit();
          }
        }}
        {...attach.handlers}
      />
      <span className="md-foot">
        <span>{attach.busy ? `正在上传 ${attach.busy} 个附件…` : '支持 Markdown；可以粘贴截图、拖入文件'}</span>
        <button type="button" className="btn sm ghost" onClick={attach.pick}>
          添加附件
        </button>
      </span>
      {attach.fileInput}
    </div>
  );
}
