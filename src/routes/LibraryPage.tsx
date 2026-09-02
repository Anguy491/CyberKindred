import type { RefObject } from "react";

import { ControlButton } from "../components/ControlButton";

interface LibraryPageProps {
  readonly permissionDenied: boolean;
  readonly query: string;
  readonly searchRef: RefObject<HTMLInputElement | null>;
  readonly onQueryChange: (value: string) => void;
}

export function LibraryPage({
  permissionDenied,
  query,
  searchRef,
  onQueryChange,
}: LibraryPageProps) {
  return (
    <section className="page" aria-labelledby="library-title">
      <div className="page-primary page-primary--empty">
        <p className="instrument-label">LIBRARY / EMPTY</p>
        <h1 id="library-title" className="hero-title">还没有本地曲库</h1>
      </div>

      <div className="page-secondary library-workbench">
        {permissionDenied ? (
          <div className="inline-error" role="alert">
            <strong>[ERROR: PERMISSION DENIED]</strong>
            <span>上次选择的位置不可读取。请通过原生目录选择器重新授权。</span>
          </div>
        ) : null}
        <p className="secondary-copy">选择包含你有权使用的音乐文件的目录。</p>
        <ControlButton
          tone="primary"
          disabledReason="[UNAVAILABLE: ONBOARDING NOT COMPLETE]"
        >
          {permissionDenied ? "重新选择目录" : "选择目录"}
        </ControlButton>

        <label className="search-field" htmlFor="library-search">
          <span className="instrument-label">搜索曲库 / CTRL+K</span>
          <input
            id="library-search"
            ref={searchRef}
            type="search"
            value={query}
            placeholder="标题、艺术家或专辑"
            onChange={(event) => onQueryChange(event.target.value)}
          />
        </label>
        <p className="empty-detail">暂无曲目。目录授权和扫描将在后续步骤启用。</p>
      </div>

      <aside className="page-tertiary" aria-label="曲库状态">
        <dl className="instrument-list">
          <div><dt>ROOTS</dt><dd>0</dd></div>
          <div><dt>TRACKS</dt><dd>0</dd></div>
          <div><dt>SCAN</dt><dd>[NOT STARTED]</dd></div>
        </dl>
      </aside>
    </section>
  );
}
