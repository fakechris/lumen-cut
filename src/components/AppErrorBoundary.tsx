import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class AppErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("lumen-cut interface error", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <main className="app-error-boundary" role="alert">
        <div>
          <p className="eyebrow">lumen-cut</p>
          <h1>界面出现问题</h1>
          <p>项目数据没有丢失。重新载入应用后可以继续操作。</p>
          <small>The interface stopped unexpectedly. Your project data is safe.</small>
          <button className="button-primary" onClick={() => window.location.reload()}>
            重新载入
          </button>
        </div>
      </main>
    );
  }
}
