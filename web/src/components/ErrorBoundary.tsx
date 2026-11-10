/* 错误边界：渲染出错时显示原因和「重新加载」，不整页白屏 */
import { Component, type ErrorInfo, type ReactNode } from 'react';

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('界面渲染出错', error, info.componentStack);
  }

  render() {
    const e = this.state.error;
    if (!e) return this.props.children;
    return (
      <div className="crash" role="alert">
        <div className="crash-box">
          <b>界面出错了</b>
          <p>这一页没能画出来。已经保存的数据不受影响，重新加载一般就好了；反复出现时请把下面的原因发给维护者。</p>
          <pre>{String(e.message || e)}</pre>
          <button className="btn primary" type="button" onClick={() => location.reload()}>
            重新加载
          </button>
        </div>
      </div>
    );
  }
}
