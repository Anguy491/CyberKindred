import type { FoundationState } from "../design/foundation";

interface ShellNoticeProps {
  readonly state: FoundationState;
}

export function ShellNotice({ state }: ShellNoticeProps) {
  if (state === "offline") {
    return (
      <aside className="inline-notice" aria-label="离线状态">
        <strong>[OFFLINE]</strong>
        <span>外部增强暂停；本地曲库、播放和数据页面仍可使用。</span>
      </aside>
    );
  }
  if (state === "degraded") {
    return (
      <aside className="inline-notice inline-notice--warning" aria-label="降级状态">
        <strong>[TTS UNAVAILABLE — TEXT CONTINUES]</strong>
        <span>语音暂不可用，文字内容和本地音乐可以继续。</span>
      </aside>
    );
  }
  return null;
}
