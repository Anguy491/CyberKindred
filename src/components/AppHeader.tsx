import { ROUTES, type FoundationState, type RouteId } from "../design/foundation";

interface AppHeaderProps {
  readonly activeRoute: RouteId;
  readonly state: FoundationState;
  readonly onNavigate: (route: RouteId) => void;
}

const STATE_LABEL: Record<FoundationState, string> = {
  initializing: "[LOADING…]",
  ready: "[LOCAL READY]",
  empty: "[LOCAL READY]",
  offline: "[OFFLINE]",
  degraded: "[DEGRADED]",
  error: "[ERROR]",
  "permission-denied": "[PERMISSION LIMITED]",
};

export function AppHeader({ activeRoute, state, onNavigate }: AppHeaderProps) {
  return (
    <header className="app-header">
      <div className="brand-block">
        <span className="brand-mark" aria-hidden="true">CK</span>
        <span className="brand-name">CYBERKINDRED</span>
      </div>
      <nav className="primary-nav" aria-label="主导航">
        {ROUTES.map((route) => (
          <button
            key={route.id}
            type="button"
            className="nav-item"
            aria-label={route.accessibleName}
            aria-current={activeRoute === route.id ? "page" : undefined}
            title={`${route.accessibleName} / ${route.shortcut}`}
            onClick={() => onNavigate(route.id)}
          >
            {route.label}
          </button>
        ))}
      </nav>
      <p className={`connection-state connection-state--${state}`} role="status" aria-live="polite">
        {STATE_LABEL[state]}
      </p>
    </header>
  );
}
