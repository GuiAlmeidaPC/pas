import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(_error: Error, _info: ErrorInfo): void {
    // The fallback below keeps a render failure visible and recoverable.
    // Avoid logging potentially sensitive project content to the webview console.
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <main className="fatal-error" role="alert" aria-labelledby="fatal-error-title">
        <h1 id="fatal-error-title">PAS encountered an interface error</h1>
        <p>
          Files already saved to disk are safe. Reload the application to restore the
          workspace; unsaved editor changes may not be recoverable.
        </p>
        <details>
          <summary>Error details</summary>
          <pre>{this.state.error.message}</pre>
        </details>
        <button type="button" onClick={() => window.location.reload()}>
          Reload PAS
        </button>
      </main>
    );
  }
}
