import type { RefObject } from "react";

import { ControlButton } from "../components/ControlButton";
import type { AppCapabilities } from "../ipc";

interface RadioPageProps {
  readonly capabilities: AppCapabilities;
  readonly draft: string;
  readonly inputRef: RefObject<HTMLTextAreaElement | null>;
  readonly onDraftChange: (value: string) => void;
}

export function RadioPage({ capabilities, draft, inputRef, onDraftChange }: RadioPageProps) {
  const systemAvailable = capabilities.features.systemMediaSession;
  const engineReason = "[UNAVAILABLE: PROGRAM ENGINE NOT READY]";
  return (
    <section className="page" aria-labelledby="radio-title">
      <div className="page-primary" aria-live="polite">
        <p className="instrument-label">RADIO / IDLE</p>
        <h1 id="radio-title" className="hero-title">今天想听什么状态？</h1>
      </div>

      <div className="page-secondary radio-workbench">
        <fieldset className="source-selector">
          <legend className="instrument-label">音乐来源</legend>
          <ControlButton aria-pressed="true" disabledReason={engineReason}>LOCAL FILES</ControlButton>
          <ControlButton
            aria-pressed="false"
            disabledReason={systemAvailable ? engineReason : "[UNAVAILABLE: APPLE MUSIC SESSION]"}
          >
            APPLE MUSIC / WINDOWS APP
          </ControlButton>
        </fieldset>

        <div className="primary-action-row">
          <ControlButton tone="primary" disabledReason={engineReason}>开始节目</ControlButton>
          <span className="inline-status">保持静音，等待用户明确开始。</span>
        </div>

        <div className="playback-controls" aria-label="播放控制">
          <ControlButton disabledReason={engineReason}>上一首</ControlButton>
          <ControlButton disabledReason={engineReason}>播放</ControlButton>
          <ControlButton disabledReason={engineReason}>下一首</ControlButton>
          <label className="progress-control">
            <span className="instrument-label">进度 / 00:00</span>
            <input type="range" min="0" max="100" value="0" disabled aria-label="播放进度，不可用" readOnly />
          </label>
        </div>

        <label className="text-entry" htmlFor="radio-message">
          <span className="instrument-label">告诉 CyberKindred 你现在想听什么</span>
          <textarea
            id="radio-message"
            ref={inputRef}
            rows={2}
            value={draft}
            placeholder="例如：安静一点，放些适合晚上的音乐"
            onChange={(event) => onDraftChange(event.target.value)}
          />
        </label>
        <div className="inline-action-row">
          <ControlButton disabledReason={engineReason}>发送</ControlButton>
          <span className="instrument-label">ENTER 发送 / SHIFT+ENTER 换行</span>
        </div>
      </div>

      <aside className="page-tertiary" aria-label="电台状态">
        <dl className="instrument-list">
          <div><dt>NOW</dt><dd>尚未开始节目</dd></div>
          <div><dt>WEATHER</dt><dd>尚未配置</dd></div>
          <div><dt>NEXT SCHEDULE</dt><dd>尚未设置</dd></div>
          <div><dt>SESSION USAGE</dt><dd>LLM 0 / 0 · TTS 0</dd></div>
        </dl>
      </aside>
    </section>
  );
}
