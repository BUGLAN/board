/* 查看文本类附件（default/main#287）：评论里的文件胶囊、卡片详情的附件列表点开 .txt / .log / .md / .json / 源代码……
   等宽字体、保留换行；能复制全部、下载，Esc 关闭。只读前 256 KB（lib/textFile），大日志不会卡住界面。
   网页、桌面端一样：同源读取，不开新窗口 */
import { useEffect, useRef, useState } from 'react';
import { fileSize } from '../../lib/attach';
import { readHead, TEXT_LIMIT, type TextHead } from '../../lib/textFile';
import { copyText, cx } from '../../lib/util';
import { closeTextFile, toast, useUI } from '../../store/ui';
import { Icon } from '../Icon';
import { Skeleton } from '../Skeleton';

type Loaded = { url: string; head?: TextHead; error?: string };

async function fetchFile(url: string, signal?: AbortSignal) {
  const res = await fetch(url, { credentials: 'same-origin', signal });
  if (!res.ok) throw new Error(res.status === 404 ? '附件不存在，可能已经删除了' : `读取失败（${res.status}）`);
  return res;
}

export function TextViewer() {
  const f = useUI(s => s.textFile);
  const [st, setSt] = useState<Loaded | null>(null);
  const [copying, setCopying] = useState(false);
  const pre = useRef<HTMLPreElement>(null);
  const url = f?.url;

  useEffect(() => {
    if (!url) return;
    const ac = new AbortController();
    setSt({ url });
    fetchFile(url, ac.signal)
      .then(res => readHead(res))
      .then(head => !ac.signal.aborted && setSt({ url, head }))
      .catch((e: Error) => !ac.signal.aborted && setSt({ url, error: e?.message || '读取失败' }));
    return () => ac.abort();
  }, [url]);

  useEffect(() => {
    if (!url) return;
    pre.current?.focus({ preventScroll: true });
    // 捕获阶段处理：Esc 只关查看器，不关下面的卡片详情
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      closeTextFile();
    };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [url]);

  if (!f) return null;
  const cur = st?.url === f.url ? st : null;
  const head = cur?.head;
  const lines = head ? head.text.split('\n').length - (head.text.endsWith('\n') ? 1 : 0) : 0;

  // 复制全部：只显示了前一部分时，重新读整个文件再复制
  const copy = async () => {
    if (!head || copying) return;
    setCopying(true);
    try {
      const text = head.truncated ? await (await fetchFile(f.url)).text() : head.text;
      const ok = await copyText(text);
      toast(ok ? { type: 'success', title: '已复制', text: `${f.name} 的全部内容`, duration: 1800 } : { type: 'error', title: '复制失败', text: '浏览器不让写剪贴板，可以下载后打开' });
    } catch (e) {
      toast({ type: 'error', title: '复制失败', text: (e as Error)?.message || '读取失败' });
    } finally {
      setCopying(false);
    }
  };

  return (
    <div className="tv-wrap" role="dialog" aria-modal="true" aria-label={f.name} onClick={closeTextFile}>
      <div className="tv" onClick={e => e.stopPropagation()}>
        <div className="tv-head">
          <Icon name="file" />
          <b className="tv-name" title={f.name}>
            {f.name}
          </b>
          {head && (
            <span className="tv-meta mono">
              {head.truncated ? `显示前 ${lines} 行 · ${fileSize(head.bytes)}` : `${lines} 行 · ${fileSize(head.bytes)}`}
            </span>
          )}
          <span className="spacer" />
          <button type="button" className={cx('btn sm ghost', copying && 'is-busy')} disabled={!head || copying} onClick={() => void copy()}>
            <Icon name="copy" />
            <span className="tv-bl">复制全部</span>
          </button>
          <a className="btn sm ghost" href={f.url} download={f.name}>
            <Icon name="download" />
            <span className="tv-bl">下载</span>
          </a>
          <button type="button" className="icon-btn" aria-label="关闭" data-tip="关闭  esc" data-tip-align="end" onClick={closeTextFile}>
            <Icon name="close" />
          </button>
        </div>
        {head?.truncated && (
          <div className="tv-note" role="note">
            <Icon name="info" />
            文件较大，只显示前 {fileSize(TEXT_LIMIT)}；完整内容请下载。
          </div>
        )}
        {cur?.error ? (
          <div className="tv-empty" role="alert">
            <b>打不开这个文件</b>
            <span>{cur.error}</span>
          </div>
        ) : !head ? (
          <div className="tv-loading" aria-busy="true">
            {[62, 48, 74, 36, 58].map((w, i) => (
              <Skeleton key={i} w={`${w}%`} h={11} />
            ))}
          </div>
        ) : head.text ? (
          <pre className="tv-body" ref={pre} tabIndex={0}>
            {head.text}
          </pre>
        ) : (
          <div className="tv-empty">
            <b>空文件</b>
          </div>
        )}
      </div>
    </div>
  );
}
