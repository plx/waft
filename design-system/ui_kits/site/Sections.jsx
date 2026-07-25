// Feature grid + Docs preview grid — landing page mid-sections.

function FeatureGrid({ onNavigate }) {
  const features = [
    {
      idx: "01",
      eyebrow: "Quick start",
      title: "Plan before copying",
      body: "Add .worktreeinclude, run waft copy --dry-run, inspect the plan, then run waft copy.",
      cta: "Read usage",
      screen: "usage",
    },
    {
      idx: "02",
      eyebrow: "Include format",
      title: "Select ignored files with Gitignore-style patterns",
      body: ".worktreeinclude chooses the ignored, untracked regular files that should move between worktrees.",
      cta: "Read format",
      screen: "worktreeinclude",
    },
    {
      idx: "03",
      eyebrow: "Safety",
      title: "Tracked files and symlinks stay protected",
      body: "waft refuses tracked overwrites, source symlinks, and symlinked destination parents. Existing files require --overwrite.",
      cta: "Read safety",
      screen: "safety",
    },
  ];
  return (
    <section className="section section--muted">
      <div className="section-shell">
        <div className="section-heading">
          <h2>A plan-then-execute file tool for worktrees.</h2>
        </div>
        <div className="feature-grid">
          {features.map((f) => (
            <article key={f.idx} className="feature-card">
              <span className="feature-card__index">{f.idx}</span>
              <p className="feature-card__eyebrow">{f.eyebrow}</p>
              <h3>{f.title}</h3>
              <p>{f.body}</p>
              <a
                href={"#" + f.screen}
                className="text-link"
                onClick={(e) => {
                  e.preventDefault();
                  onNavigate(f.screen);
                }}
              >
                {f.cta} →
              </a>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

function DocsPreview({ onNavigate }) {
  const docs = [
    { slug: "usage", title: "Usage", body: "Commands, global options, copy options, typical flow." },
    { slug: "worktreeinclude", title: ".worktreeinclude", body: "Include file format, eligibility, profile-dependent matching." },
    { slug: "profiles", title: "Profiles", body: "claude / git / wt — coordinated compatibility presets." },
    { slug: "safety", title: "Safety", body: "Guarantees: tracked-never-overwritten, symlink-blocked, atomic." },
    { slug: "configuration", title: "Configuration", body: "Layered config from defaults → user → project → env → CLI." },
    { slug: "architecture", title: "Architecture", body: "Git is authoritative; matching is pluggable; plan-then-execute." },
  ];
  return (
    <section className="section">
      <div className="section-shell">
        <div className="section-heading">
          <h2>Documentation</h2>
        </div>
        <div className="docs-grid">
          {docs.map((d) => (
            <article key={d.slug} className="doc-card">
              <h3>{d.title}</h3>
              <p>{d.body}</p>
              <a
                href={"#" + d.slug}
                className="text-link"
                onClick={(e) => {
                  e.preventDefault();
                  onNavigate(d.slug);
                }}
              >
                Read →
              </a>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

Object.assign(window, { FeatureGrid, DocsPreview });
