// Hero — landing-page first fold. Two-column on wide, stacked on narrow.

function TerminalCard({ title = "waft", meta = "copy commands", lines, copyText }) {
  const [copied, setCopied] = React.useState(false);
  const onCopy = () => {
    try {
      navigator.clipboard?.writeText(copyText || lines.join("\n"));
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch (_) {}
  };
  return (
    <div className="command-card">
      <div className="command-card__header">
        <div>
          <p className="eyebrow">Terminal</p>
          <h2>{title}</h2>
        </div>
        <span>{meta}</span>
      </div>
      <pre className="command-card__body">
        <code>{lines.join("\n")}</code>
      </pre>
      <button className="copy-button" onClick={onCopy} type="button">
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function TransferPanel() {
  const includes = [".env", "*.env.local", "**/*.key"];
  return (
    <aside className="transfer-panel">
      <h2>What gets copied</h2>
      <p>
        <code>.worktreeinclude</code> selects files using familiar gitignore
        syntax.
      </p>
      <div className="transfer-list">
        {includes.map((s) => (
          <span key={s}>{s}</span>
        ))}
      </div>
      <div className="transfer-rule">
        <strong>! test.key</strong>
        <span>Negations remove a path from the include set.</span>
      </div>
    </aside>
  );
}

function Hero({ onNavigate }) {
  return (
    <section className="section-shell hero" id="overview">
      <div className="hero__copy">
        <p className="eyebrow">Worktree-aware file tool</p>
        <h1>Plan before copying.</h1>
        <p className="hero__lede">
          <code>waft</code> copies <code>.worktreeinclude</code>-selected ignored
          files between Git worktrees.
        </p>
        <p className="hero__body">
          A small Rust CLI for copying selected ignored files — env files, API
          keys, build caches — between linked worktrees, with a plan-then-execute
          safety model.
        </p>
        <div className="badge-row">
          <span className="badge">Rust</span>
          <span className="badge">CLI</span>
          <span className="badge">Git worktrees</span>
        </div>
        <div className="hero__actions">
          <a
            href="#usage"
            className="button button--primary"
            onClick={(e) => {
              e.preventDefault();
              onNavigate("usage");
            }}
          >
            Read the docs
          </a>
          <a
            href="https://github.com/plx/waft"
            className="button button--secondary"
          >
            View on GitHub
          </a>
        </div>
      </div>
      <div className="hero__visual">
        <TerminalCard
          lines={[
            "$ waft copy --dry-run",
            "$ waft copy --source ../main --dest .",
          ]}
          copyText={"waft copy --dry-run\nwaft copy --source ../main --dest ."}
        />
        <TransferPanel />
      </div>
    </section>
  );
}

Object.assign(window, { Hero, TerminalCard, TransferPanel });
