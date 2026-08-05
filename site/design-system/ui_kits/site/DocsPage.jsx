// Docs page — Starlight-style sidebar + main column.
// Renders the actual MDX docs as JSX (content drawn from site/src/content/docs/*.mdx).

const DOC_PAGES = [
  { slug: "usage", title: "Usage", lede: "Copy ignored files selected by .worktreeinclude between Git worktrees." },
  { slug: "worktreeinclude", title: ".worktreeinclude", lede: "The include file format used by waft." },
  { slug: "safety", title: "Safety", lede: "The guarantees waft keeps while copying ignored worktree files." },
  { slug: "profiles", title: "Profiles", lede: "Compatibility profiles for different worktree workflows." },
  { slug: "configuration", title: "Configuration", lede: "Layered configuration and per-knob overrides." },
  { slug: "architecture", title: "Architecture", lede: "How waft plans and executes safe worktree file copies." },
];

function Sidebar({ active, onNavigate }) {
  return (
    <aside className="docs__sidebar" aria-label="Docs">
      <p className="docs__sidebar-heading">Guides</p>
      <ul>
        {DOC_PAGES.map((p) => (
          <li key={p.slug}>
            <a
              href={"#" + p.slug}
              className={active === p.slug ? "is-active" : ""}
              onClick={(e) => {
                e.preventDefault();
                onNavigate(p.slug);
              }}
            >
              {p.title}
            </a>
          </li>
        ))}
      </ul>
    </aside>
  );
}

function CodeBlock({ children, lang }) {
  return (
    <pre>
      <code className={lang ? "lang-" + lang : ""}>{children}</code>
    </pre>
  );
}

// ── Page bodies ─────────────────────────────────────────────────────────────

function UsagePage() {
  return (
    <>
      <h1>Usage</h1>
      <p className="lede">Copy ignored files selected by <code>.worktreeinclude</code> between Git worktrees.</p>
      <p><code>waft</code> copies <code>.worktreeinclude</code>-selected ignored files between Git worktrees. It is intended for local configuration, caches, and tool state that Git should not track but that developers often need in more than one linked worktree.</p>

      <h2>Quick start</h2>
      <CodeBlock lang="sh">{`# In a linked worktree, copy from the main worktree automatically.
waft

# Explicit source and destination.
waft copy --source /path/to/main --dest /path/to/linked

# See what would be copied without writing files.
waft copy --dry-run

# List eligible files.
waft list

# Inspect specific files.
waft info .env

# Validate ignore files.
waft validate`}</CodeBlock>

      <h2>Commands</h2>
      <table>
        <thead><tr><th>Command</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>waft</code> / <code>waft copy</code></td><td>Copy eligible files. This is the default command.</td></tr>
          <tr><td><code>waft list</code></td><td>List eligible files without copying.</td></tr>
          <tr><td><code>waft info &lt;PATH&gt;...</code></td><td>Show detailed status for specific files.</td></tr>
          <tr><td><code>waft validate</code></td><td>Check ignore files for syntax errors.</td></tr>
        </tbody>
      </table>

      <h2>Global options</h2>
      <table>
        <thead><tr><th>Option</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>--source &lt;PATH&gt;</code></td><td>Source, usually the main worktree path.</td></tr>
          <tr><td><code>--dest &lt;PATH&gt;</code></td><td>Destination, usually the linked worktree path.</td></tr>
          <tr><td><code>-C &lt;PATH&gt;</code></td><td>Operate as if started in <code>PATH</code>.</td></tr>
          <tr><td><code>-q, --quiet</code></td><td>Suppress non-error output.</td></tr>
          <tr><td><code>-v, --verbose</code></td><td>Increase output verbosity.</td></tr>
        </tbody>
      </table>

      <h2>Typical flow</h2>
      <ol>
        <li>Add <code>.worktreeinclude</code> to the source repo.</li>
        <li>Run <code>waft copy --dry-run</code>.</li>
        <li>Inspect the plan.</li>
        <li>Run <code>waft copy</code>.</li>
        <li>Use <code>waft info &lt;path&gt;</code> when a file is missing or skipped.</li>
      </ol>
    </>
  );
}

function WorktreeIncludePage() {
  return (
    <>
      <h1>.worktreeinclude</h1>
      <p className="lede">The include file format used by waft.</p>
      <p><code>.worktreeinclude</code> uses familiar <code>.gitignore</code> syntax to select ignored files that <code>waft</code> should copy between worktrees.</p>
      <CodeBlock>{`# Include environment files
.env
*.env.local

# Include all secret keys recursively
**/*.key

# But not test keys
!test.key`}</CodeBlock>

      <h2>Eligibility</h2>
      <p>A file is eligible for copying only when all of these are true:</p>
      <ol>
        <li>It exists in the source worktree.</li>
        <li>It is a regular file, not a symlink, directory, or special file.</li>
        <li>It is selected by the active compatibility profile.</li>
        <li>Git confirms it is ignored and untracked.</li>
        <li>It is not dropped by the active exclusion set.</li>
      </ol>

      <h2>Profile-dependent matching</h2>
      <p>By default, the <code>claude</code> profile only consults the root-level <code>.worktreeinclude</code>.</p>
      <p>Use <code>--compat-profile git</code> when you want nested <code>.worktreeinclude</code> files to compose like nested <code>.gitignore</code> files. Use <code>--compat-profile wt</code> when you want worktrunk-compatible all-ignored behavior.</p>

      <h2>Validation</h2>
      <CodeBlock lang="sh">{`waft validate`}</CodeBlock>
      <p>Validation parses ignore files before copy execution. Copy planning remains read-only until the command enters the execute phase.</p>
    </>
  );
}

function SafetyPage() {
  return (
    <>
      <h1>Safety</h1>
      <p className="lede">The guarantees waft keeps while copying ignored worktree files.</p>
      <p><code>waft</code> is designed around a plan-then-execute pipeline. Discovery and classification happen before any filesystem mutation.</p>

      <h2>Guarantees</h2>
      <ul>
        <li><strong>Tracked files are never overwritten.</strong> Destination trackedness is checked before copy execution.</li>
        <li><strong>Symlink traversal is blocked.</strong> <code>waft</code> refuses to follow symlinks in source files or write through symlinked destination parents.</li>
        <li><strong>Atomic writes.</strong> Files are written to a temporary path and then renamed into place.</li>
        <li><strong>Dry runs are mutation-free.</strong> <code>waft copy --dry-run</code> reads and reports only.</li>
      </ul>

      <h2>Preview before writing</h2>
      <CodeBlock lang="sh">{`waft copy --dry-run`}</CodeBlock>
      <p>Use dry-run output to confirm which files would move and why.</p>

      <h2>Overwrite behavior</h2>
      <p>Existing destination files are preserved by default. Use <code>--overwrite</code> only when you intentionally want untracked destination files replaced.</p>
      <CodeBlock lang="sh">{`waft copy --overwrite`}</CodeBlock>
      <p><code>--overwrite</code> does not change the rule that tracked files are protected.</p>
    </>
  );
}

function ProfilesPage() {
  return (
    <>
      <h1>Profiles</h1>
      <p className="lede">Compatibility profiles for different worktree workflows.</p>
      <p><code>waft</code> ships with coordinated compatibility profiles. Each profile selects a bundle of matcher and safety behavior.</p>

      <table>
        <thead><tr><th>Profile</th><th>Missing <code>.worktreeinclude</code></th><th>Matcher semantics</th><th>Symlinked rule files</th><th>Tool-state excludes</th></tr></thead>
        <tbody>
          <tr><td><code>claude</code></td><td>nothing selected</td><td><code>claude-2026-04</code></td><td>follow</td><td>none</td></tr>
          <tr><td><code>git</code></td><td>nothing selected</td><td><code>git</code></td><td>ignore</td><td>none</td></tr>
          <tr><td><code>wt</code></td><td>every git-ignored untracked file selected</td><td><code>wt-0.39</code></td><td>follow</td><td><code>tooling-v1</code></td></tr>
        </tbody>
      </table>

      <h2><code>claude</code></h2>
      <p>The default profile. It matches Claude Code's out-of-the-box behavior by using the repository root <code>.worktreeinclude</code>.</p>
      <CodeBlock lang="sh">{`waft copy --compat-profile claude`}</CodeBlock>

      <h2><code>git</code></h2>
      <p>Use this profile when you want Git-style per-directory exclude semantics, where patterns are relative to the directory containing the file and deeper files can override shallower files.</p>
      <CodeBlock lang="sh">{`waft copy --compat-profile git`}</CodeBlock>

      <h2><code>wt</code></h2>
      <p>Use this profile for worktrunk parity. It starts from every git-ignored untracked file and removes paths matched by literal-name negations.</p>
      <CodeBlock lang="sh">{`waft copy --compat-profile wt`}</CodeBlock>
    </>
  );
}

function ConfigurationPage() {
  return (
    <>
      <h1>Configuration</h1>
      <p className="lede">Layered configuration and per-knob overrides.</p>
      <p><code>waft</code> resolves profile and policy settings from multiple layers. Later layers win for scalar values.</p>
      <ol>
        <li>Built-in defaults.</li>
        <li>User config: <code>~/.config/waft/config.toml</code>.</li>
        <li>Project configs: each <code>.waft.toml</code> from repo root down to the current directory.</li>
        <li>Environment variables using the <code>WAFT_*</code> prefix.</li>
        <li>CLI flags.</li>
      </ol>
      <p>Explicit knob settings always beat preset values from a higher-precedence profile layer.</p>

      <h2>Example <code>.waft.toml</code></h2>
      <CodeBlock lang="toml">{`version = 1

[compat]
profile = "git"

[exclude]
extra = ["*.bak"]`}</CodeBlock>
    </>
  );
}

function ArchitecturePage() {
  return (
    <>
      <h1>Architecture</h1>
      <p className="lede">How waft plans and executes safe worktree file copies.</p>
      <p><code>waft</code> is built around a few invariant design decisions.</p>

      <h2>Git is authoritative</h2>
      <p><code>waft</code> does not reimplement Git's ignore logic for final ignored/tracked decisions. Those checks go through the <code>GitBackend</code> trait.</p>
      <ul>
        <li><code>GitGix</code> is the default in-process backend built on the <code>gix</code> crate.</li>
        <li><code>GitCli</code> shells out to Git and is selected with <code>WAFT_GIT_BACKEND=cli</code>.</li>
      </ul>

      <h2>Commands plan before executing</h2>
      <p>All commands follow a read-only planning pipeline before writes are possible.</p>
      <ol>
        <li>Parse CLI arguments.</li>
        <li>Resolve layered policy.</li>
        <li>Resolve source and destination worktree context.</li>
        <li>Validate ignore files.</li>
        <li>Select candidate files.</li>
        <li>Apply policy filters.</li>
        <li>Confirm ignored status through Git.</li>
        <li>Build a copy plan.</li>
        <li>Execute only for <code>copy</code> without <code>--dry-run</code>.</li>
      </ol>
    </>
  );
}

const PAGE_BY_SLUG = {
  usage: UsagePage,
  worktreeinclude: WorktreeIncludePage,
  safety: SafetyPage,
  profiles: ProfilesPage,
  configuration: ConfigurationPage,
  architecture: ArchitecturePage,
};

function DocsPage({ slug, onNavigate }) {
  const Page = PAGE_BY_SLUG[slug] || UsagePage;
  return (
    <div className="section-shell docs">
      <Sidebar active={slug} onNavigate={onNavigate} />
      <main className="docs__main" data-screen-label={`docs/${slug}`}>
        <Page />
      </main>
    </div>
  );
}

Object.assign(window, { DocsPage, DOC_PAGES });
