/* ==========================================================================
   附件上传（PRD §5）：描述、评论的输入框里粘贴截图、拖入文件、点按钮选文件
   - 上传时先在光标处插入占位文字，完成后换成 Markdown（图片 ![名](地址)，文件 [名](地址)）
   - 失败时去掉占位并提示；类型、大小先在本地检查，服务端会再检查一次
   - 新建弹窗里还没有卡片：先上传，新建时服务端按描述里的引用把附件归到这张卡片
   ========================================================================== */
import { useRef, useState, type ClipboardEvent, type Dispatch, type DragEvent, type RefObject, type SetStateAction } from 'react';
import { toast, toastError } from '../store/ui';
import { api } from './api';
import { autosize } from './util';

export const MAX_FILE = 10 * 1024 * 1024;
export const ACCEPT = '.png,.jpg,.jpeg,.gif,.webp,.pdf,.txt,.log,.md,.markdown,.json,.csv,.zip';
const EXT = /\.(png|jpe?g|gif|webp|pdf|txt|log|md|markdown|json|csv|zip)$/i;
const IMAGE_EXT: Record<string, string> = { 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' };

export function fileSize(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

/** 粘贴的截图常常叫 image.png 或没有扩展名：按类型补一个带时间的名字 */
function named(f: File): File {
  const ext = IMAGE_EXT[f.type];
  if (EXT.test(f.name) && !/^image\.(png|jpe?g|gif|webp)$/i.test(f.name)) return f;
  if (!ext) return f;
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  const stamp = `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
  return new File([f], `截图-${stamp}.${ext}`, { type: f.type });
}

let seq = 0;

interface Opts {
  /** 白板路径（项目/白板） */
  board: string | null | undefined;
  /** 卡片 ID；新建弹窗里为 null */
  card: number | null;
  setValue: Dispatch<SetStateAction<string>>;
  textarea: RefObject<HTMLTextAreaElement | null>;
}

export function useAttach(o: Opts) {
  const latest = useRef(o);
  latest.current = o;
  const [busy, setBusy] = useState(0);
  const [over, setOver] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  /** 在光标处插入（单独成行），光标移到插入内容之后 */
  const insert = (text: string) => {
    const t = latest.current.textarea.current;
    latest.current.setValue(v => {
      const start = t?.selectionStart ?? v.length;
      const end = t?.selectionEnd ?? v.length;
      const before = v.slice(0, start);
      const after = v.slice(end);
      const pre = before && !before.endsWith('\n') ? '\n' : '';
      const post = after && !after.startsWith('\n') ? '\n' : '';
      const pos = (before + pre + text).length;
      requestAnimationFrame(() => {
        if (!t) return;
        t.focus();
        t.setSelectionRange(pos, pos);
        autosize(t);
      });
      return before + pre + text + post + after;
    });
  };

  const replace = (mark: string, md: string) => {
    latest.current.setValue(v => {
      if (md) return v.replace(mark, md);
      // 失败：连同占位所在的换行一起去掉
      return v.replace(`${mark}\n`, '').replace(`\n${mark}`, '').replace(mark, '');
    });
    requestAnimationFrame(() => autosize(latest.current.textarea.current));
  };

  const upload = async (raw: File[]) => {
    const board = latest.current.board;
    if (!board || !raw.length) return;
    const files = raw.map(named);
    const bad = files.filter(f => !EXT.test(f.name));
    if (bad.length)
      toast({
        type: 'error',
        title: '不支持这种文件',
        text: `${bad.map(f => f.name || '（没有名字）').join('、')}。支持 PNG、JPEG、GIF、WebP 图片，PDF、TXT、Markdown、日志、JSON、CSV、ZIP。`,
      });
    const big = files.filter(f => EXT.test(f.name) && f.size > MAX_FILE);
    if (big.length) toast({ type: 'error', title: '附件不能超过 10 MB', text: big.map(f => `${f.name}（${fileSize(f.size)}）`).join('、') });
    const ok = files.filter(f => EXT.test(f.name) && f.size > 0 && f.size <= MAX_FILE);
    if (!ok.length) return;
    const marks = ok.map(f => `![上传中：${f.name.replace(/[[\]]/g, ' ')}…](#upload-${++seq})`);
    insert(marks.join('\n'));
    setBusy(b => b + ok.length);
    await Promise.all(
      ok.map(async (f, i) => {
        try {
          const a = await api.upload(board, latest.current.card, f);
          replace(marks[i], a.markdown);
        } catch (e) {
          replace(marks[i], '');
          toastError(e);
        } finally {
          setBusy(b => b - 1);
        }
      }),
    );
  };

  const handlers = {
    onPaste: (e: ClipboardEvent<HTMLTextAreaElement>) => {
      const files = [...e.clipboardData.files];
      if (!files.length) return;
      e.preventDefault();
      upload(files);
    },
    onDragOver: (e: DragEvent<HTMLTextAreaElement>) => {
      if (![...e.dataTransfer.types].includes('Files')) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
      if (!over) setOver(true);
    },
    onDragLeave: () => setOver(false),
    onDrop: (e: DragEvent<HTMLTextAreaElement>) => {
      setOver(false);
      if (!e.dataTransfer.files.length) return;
      e.preventDefault();
      upload([...e.dataTransfer.files]);
    },
  };

  const fileInput = (
    <input
      ref={input}
      type="file"
      multiple
      accept={ACCEPT}
      hidden
      onChange={e => {
        upload([...(e.target.files ?? [])]);
        e.target.value = '';
      }}
    />
  );

  return { busy, over, handlers, fileInput, pick: () => input.current?.click() };
}

/** 还在上传时不能提交：提示一下 */
export function stillUploading(busy: number) {
  if (busy <= 0) return false;
  toast({ type: 'info', title: '附件还在上传', text: '上传完再保存，描述里的占位会自动换成附件。', duration: 2400 });
  return true;
}
