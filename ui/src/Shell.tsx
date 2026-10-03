import type { FormEvent, ReactNode } from "react";

import { Icon, type IconName } from "./icons";

export type View = "library" | "download" | "activity" | "review" | "sources" | "settings";

/** The vault folder, typed or picked with the desktop's dialog, opened or created. */
export function VaultForm({
  vaultRoot,
  loading,
  onChange,
  onOpen,
  onChoose,
}: {
  vaultRoot: string;
  loading: boolean;
  onChange: (vaultRoot: string) => void;
  onOpen: () => void;
  onChoose: () => void;
}) {
  return (
    <form
      className="vault-form"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        onOpen();
      }}
    >
      <label htmlFor="vault-root">Vault folder</label>
      <div className="vault-row">
        <input
          id="vault-root"
          value={vaultRoot}
          onChange={(event) => onChange(event.target.value)}
          spellCheck={false}
        />
        <button type="button" disabled={loading} onClick={onChoose}>
          <Icon name="folder" size={18} />
          Choose vault folder…
        </button>
        <button
          type="submit"
          className="primary"
          disabled={loading || vaultRoot.trim().length === 0}
        >
          {loading ? "Opening…" : "Open vault"}
        </button>
      </div>
      <p className="hint">
        Where your media are kept. A name alone is a folder in your Documents; a folder without a
        vault gets a new one. The app opens it again next time.
      </p>
    </form>
  );
}

/** The first screen: where the collection lives, and what the app then does. */
export function Welcome({ error, children }: { error: string | null; children: ReactNode }) {
  return (
    <main className="welcome">
      <section className="welcome-card" aria-label="Welcome">
        <div className="brand">
          <span className="brand-mark">
            <Icon name="gamepad" size={22} />
          </span>
          <div>
            <div className="brand-name">Game Media Vault</div>
            <div className="brand-tagline">Every cover, disc, manual and more</div>
          </div>
        </div>
        <div className="stack">
          <h1>Your game media, collected for you</h1>
          <p className="lede">
            Pick consoles, and the app finds their games and gathers every kind of media from
            every Source into one vault on your disk.
          </p>
        </div>
        {children}
        {error ? <p className="error-message">{error}</p> : null}
        <div className="welcome-steps">
          <div className="welcome-step">
            <strong>1 · Open a vault</strong>
            <span>One folder keeps everything, without duplicates.</span>
          </div>
          <div className="welcome-step">
            <strong>2 · Download</strong>
            <span>Choose consoles, regions and media, then start.</span>
          </div>
          <div className="welcome-step">
            <strong>3 · Browse</strong>
            <span>Watch covers arrive, then export them to a folder.</span>
          </div>
        </div>
      </section>
    </main>
  );
}

interface NavEntry {
  view: View;
  label: string;
  icon: IconName;
  /** A count shown beside the label, and spoken with it. */
  count?: number;
  /** Whether the count is of work under way. */
  live?: boolean;
}

/** The app around its views: navigation, the open vault, and the active page. */
export function AppShell({
  view,
  onNavigate,
  libraryCount,
  reviewCount,
  activeDownloads,
  vaultRoot,
  opening,
  onChangeVault,
  children,
}: {
  view: View;
  onNavigate: (view: View) => void;
  libraryCount: number;
  reviewCount: number;
  /** Downloads executing or waiting. */
  activeDownloads: number;
  vaultRoot: string;
  /** Whether the vault is opening, which the page under way says to assistive technologies. */
  opening: boolean;
  onChangeVault: () => void;
  children: ReactNode;
}) {
  const entries: NavEntry[] = [
    { view: "library", label: "Library", icon: "library", count: libraryCount },
    { view: "download", label: "Download", icon: "download" },
    {
      view: "activity",
      label: "Activity",
      icon: "activity",
      count: activeDownloads > 0 ? activeDownloads : undefined,
      live: true,
    },
    { view: "review", label: "Review", icon: "review", count: reviewCount },
    { view: "sources", label: "Sources", icon: "sources" },
    { view: "settings", label: "Settings", icon: "settings" },
  ];
  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">
            <Icon name="gamepad" size={20} />
          </span>
          <div>
            <div className="brand-name">Game Media Vault</div>
            <div className="brand-tagline">Local media library</div>
          </div>
        </div>
        <nav className="nav" aria-label="Vault views">
          {entries.map((entry) => (
            <button
              type="button"
              key={entry.view}
              className={view === entry.view ? "nav-item active" : "nav-item"}
              aria-current={view === entry.view ? "page" : undefined}
              // Counted views say their count, as `Library (12)`.
              aria-label={
                entry.count === undefined || entry.live
                  ? entry.label
                  : `${entry.label} (${entry.count})`
              }
              onClick={() => onNavigate(entry.view)}
            >
              <Icon name={entry.icon} />
              {entry.label}
              {entry.count !== undefined ? (
                <span className={entry.live ? "nav-badge live" : "nav-badge"}>{entry.count}</span>
              ) : null}
            </button>
          ))}
        </nav>
        <div className="vault-card">
          <span className="vault-card-label">{opening ? "Opening vault…" : "Vault"}</span>
          <span className="vault-card-path" title={vaultRoot}>
            {vaultRoot}
          </span>
          <button type="button" className="ghost" onClick={onChangeVault}>
            <Icon name="folder" size={16} />
            Change vault
          </button>
        </div>
      </aside>
      <main className="main" aria-busy={opening}>
        {children}
      </main>
    </div>
  );
}

/** A page of the app: its title, what it is for, its actions, and its content. */
export function Page({
  title,
  subtitle,
  actions,
  children,
}: {
  title: string;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="page">
      <header className="page-header">
        <div>
          <h1>{title}</h1>
          {subtitle ? <p>{subtitle}</p> : null}
        </div>
        {actions ? <div className="page-actions">{actions}</div> : null}
      </header>
      {children}
    </div>
  );
}
