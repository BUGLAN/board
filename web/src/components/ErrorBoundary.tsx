/* 错误边界：渲染出错时显示原因、出错的组件和「重新加载」，不整页白屏。
   原因和组件栈能一键复制：界面上只看到一张截图时，也能据此定位是哪个组件、哪份数据崩的 */
import { Component, useState, type ErrorInfo, type ReactNode } from 'react';
import { copyText } from '../lib/util';

interface State {
  error: Error | null;
  /** React 报告的是哪个组件栈崩的（开发版、生产版都有，每行一个组件） */
  stack: string;
}

/** 组件栈只留前几行：够认出是哪个组件，又不至于把整页撑满（空行去掉）
    React 自己的几帧（错误边界、Suspense）放在最后几行，先留着，认不出时也有得看 */
const STACK_LINES = 8;

export function componentStack(info?: string | null): string {
  const lines = String(info ?? '')
    .split('\n')
    .map(l => l.trim())
    .filter(Boolean);
  return lines.slice(0, STACK_LINES).join('\n');
}

/** 复制给维护者的内容：原因 + 组件栈 */
export function crashReport(e: Error | null, stack: string): string {
  if (!e) return '';
  return [`原因：${String(e.message || e)}`, stack && `组件：\n${stack}`].filter(Boolean).join('\n\n');
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null, stack: '' };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ stack: componentStack(info.componentStack) });
    console.error('界面渲染出错', error, info.componentStack);
  }

  render() {
    const { error, stack } = this.state;
    if (!error) return this.props.children;
    return <CrashView error={error} stack={stack} />;
  }
}

function CrashView({ error, stack }: { error: Error; stack: string }) {
  const [copied, setCopied] = useState(false);
  const report = crashReport(error, stack);
  return (
    <div className="crash" role="alert">
      <div className="crash-box">
        <b>界面出错了</b>
        <p>这一页没能画出来。已经保存的数据不受影响，重新加载一般就好了；反复出现时请把下面的原因和出错的组件发给维护者。</p>
        <pre>{String(error.message || error)}</pre>
        {stack && (
          <>
            <span className="crash-h">出错的组件</span>
            <pre className="crash-stack">{stack}</pre>
          </>
        )}
        <div className="crash-actions">
          <button className="btn primary" type="button" onClick={() => location.reload()}>
            重新加载
          </button>
          <button
            className="btn"
            type="button"
            onClick={() => void copyText(report).then(ok => setCopied(ok), () => setCopied(false))}
          >
            {copied ? '已复制' : '复制原因和组件'}
          </button>
        </div>
      </div>
    </div>
  );
}
