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
        <code>.worktreeinclude</code> patterns follow <code>.gitignore</code>{" "}
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
        <p className="eyebrow">Git worktree file copier</p>
        <h1>Copy ignored files.</h1>
        <p className="hero__lede">
          <code>waft</code> copies ignored, untracked files selected by{" "}
          <code>.worktreeinclude</code> from one Git worktree to another.
        </p>
        <p className="hero__body">
          Use it for local configuration, caches, and tool state that you need
          in more than one worktree but do not want to commit.
        </p>
        <div className="hero__actions">
          <a
            href="#usage"
            className="button button--primary"
            onClick={(e) => {
              e.preventDefault();
              onNavigate("usage");
            }}
          >
            Usage
          </a>
          <a
            href="https://github.com/plx/waft"
            className="button button--secondary"
          >
            Source
          </a>
        </div>
      </div>
      <div className="hero__visual">
        <TerminalCard
          meta="preview and copy"
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
