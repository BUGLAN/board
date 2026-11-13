/* 错误边界：渲染出错时显示原因、出错的组件和「重新加载」，不整页白屏。
   原因、组件栈能一键复制（和界面上给它看的样子一致）：截图里就带着是哪个组件出的错，不用再让人去控制台翻
   （线上报过 reading 'color'，截图只有原因，定位不到是哪个组件） */
import { Component, type ErrorInfo, type ReactNode } from 'react';
import { copyText } from '../lib/util';

/** 放在最上面几行；组件栈里靠前的就是出错的那一层，再往下是包着它的组件 */
export const STACK_LINES = 6;

interface State {
  error: Error | null;
  /** React 给的组件栈（每行一个组件，最深的在前）；没有时为空串 */
  componentStack: string;
}

/** 组件栈前几行：去掉空行和首尾空白 */
export function stackLines(componentStack: string | null | undefined, n = STACK_LINES) {
  return (componentStack ?? '')
    .split('\n')
    .map(l => l.trim())
    .filter(Boolean)
    .slice(0, n);
}

/** 「复制」给的东西：原因和组件栈都写上（和界面上一样），维护者粘出来就能看 */
export function crashText(error: Error | null, componentStack: string | null | undefined) {
  const reason = String(error?.message || error || '');
  const where = stackLines(componentStack).map(l => `  ${l}`).join('\n');
  return [`界面出错了`, `原因：${reason}`, where ? `组件：\n${where}` : '组件：不知道（没有组件栈）', `地址：${typeof location !== 'undefined' ? location.href : ''}`].filter(Boolean).join('\n');
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null, componentStack: '' };
  /** 复制了没有：按钮上短暂写「已复制」 */
  private done: number | undefined;

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('界面渲染出错', error, info.componentStack);
    this.setState({ componentStack: info.componentStack ?? '' });
  }

  componentWillUnmount() {
    clearTimeout(this.done);
  }

  copy = async (e: React.MouseEvent<HTMLButtonElement>) => {
    const btn = e.currentTarget;
    btn.textContent = (await copyText(crashText(this.state.error, this.state.componentStack))) ? '已复制' : '复制失败';
    clearTimeout(this.done);
    this.done = window.setTimeout(() => {
      if (btn.isConnected) btn.textContent = '复制原因和组件';
    }, 1600);
  };

  render() {
    const e = this.state.error;
    if (!e) return this.props.children;
    const where = stackLines(this.state.componentStack);
    return (
      <div className="crash" role="alert">
        <div className="crash-box">
          <b>界面出错了</b>
          <p>这一页没能画出来。已经保存的数据不受影响，重新加载一般就好了；反复出现时请把下面的原因和组件复制给维护者。</p>
          <pre>{String(e.message || e)}</pre>
          {where.length > 0 && (
            <pre className="crash-stack" aria-label="出错的组件">
              {where.join('\n')}
            </pre>
          )}
          <div className="crash-acts">
            <button className="btn primary" type="button" onClick={() => location.reload()}>
              重新加载
            </button>
            <button className="btn" type="button" onClick={e => void this.copy(e)}>
              复制原因和组件
            </button>
          </div>
        </div>
      </div>
    );
  }
}
