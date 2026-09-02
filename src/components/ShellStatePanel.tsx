import { ControlButton } from "./ControlButton";

interface ShellStatePanelProps {
  readonly kind: "initializing" | "error";
  readonly onRetry: () => void;
}

export function ShellStatePanel({ kind, onRetry }: ShellStatePanelProps) {
  if (kind === "initializing") {
    return (
      <section className="state-page" aria-labelledby="initializing-title">
        <p className="instrument-label">[LOADING…]</p>
        <h1 id="initializing-title" className="hero-title">正在读取本机能力</h1>
        <div className="mechanical-progress" role="progressbar" aria-label="正在初始化" aria-valuetext="正在读取本机能力">
          <span /><span /><span /><span />
        </div>
        <p className="secondary-copy">保持静音，只检查本地应用能力。</p>
      </section>
    );
  }
  return (
    <section className="state-page" aria-labelledby="error-title">
      <p className="instrument-label status-error" role="alert">[ERROR: IPC]</p>
      <h1 id="error-title" className="hero-title">无法读取本机状态</h1>
      <p className="secondary-copy">应用尚未执行播放或外部请求。请重新连接本机核心。</p>
      <ControlButton tone="primary" onClick={onRetry}>重新读取</ControlButton>
    </section>
  );
}
