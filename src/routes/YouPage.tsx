export function YouPage() {
  return (
    <section className="page" aria-labelledby="you-title">
      <div className="page-primary">
        <p className="instrument-label">YOU / LOCAL PROFILE</p>
        <h1 id="you-title" className="hero-title">CyberKindred 目前如何了解你</h1>
      </div>

      <div className="page-secondary knowledge-sections">
        <section aria-labelledby="profile-heading">
          <h2 id="profile-heading">画像</h2>
          <p>尚未完成引导；没有保存称呼或偏好。</p>
        </section>
        <section aria-labelledby="trend-heading">
          <h2 id="trend-heading">偏好趋势</h2>
          <p className="empty-detail">还没有足够的本地反馈。</p>
        </section>
        <section aria-labelledby="proposal-heading">
          <h2 id="proposal-heading">待确认记忆</h2>
          <p className="empty-detail">暂无待确认记忆。</p>
        </section>
        <section aria-labelledby="memory-heading">
          <h2 id="memory-heading">已批准记忆</h2>
          <p className="empty-detail">尚未批准任何长期记忆。</p>
        </section>
        <section aria-labelledby="summary-heading">
          <h2 id="summary-heading">会话摘要</h2>
          <p className="empty-detail">还没有长期摘要。</p>
        </section>
      </div>

      <aside className="page-tertiary" aria-label="了解数据状态">
        <dl className="instrument-list">
          <div><dt>UPDATED</dt><dd>尚未获得</dd></div>
          <div><dt>SOURCE</dt><dd>仅本机</dd></div>
          <div><dt>PROPOSALS</dt><dd>0</dd></div>
        </dl>
      </aside>
    </section>
  );
}
