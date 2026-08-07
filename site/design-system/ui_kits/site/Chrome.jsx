// Site shared layout primitives — Header (sticky nav) + Footer.
// Both are fully theme-aware (light in light mode, dark in dark mode).
// Used by every screen in the kit.

// Chip-less, theme-aware brand mark: boxed computer-fan line art —
// teal frame + rim, mustard blades. Colors from --waft-accent / --waft-mark-2
// so it reads on any surface.
function WaftMark(props) {
  return (
    <svg viewBox="0 0 80 80" role="img" aria-label="waft mark" {...props}>
      <g transform="translate(40,40) scale(0.82) translate(-40,-40)">
        <path fill="var(--waft-accent)" d="M7,67c0,3.309,2.691,6,6,6h54c3.309,0,6-2.691,6-6V13c0-3.309-2.691-6-6-6H13c-3.309,0-6,2.691-6,6V67z M11,13c0-1.103,0.897-2,2-2h54c1.103,0,2,0.897,2,2v54c0,1.103-0.897,2-2,2H13c-1.103,0-2-0.897-2-2V13z" />
        <path fill="var(--waft-accent)" d="M40,67c14.888,0,27-12.112,27-27S54.888,13,40,13S13,25.112,13,40S25.112,67,40,67z M40,17c12.683,0,23,10.318,23,23S52.683,63,40,63S17,52.682,17,40S27.317,17,40,17z" />
        <path fill="var(--waft-mark-2)" d="M32.091,41.143c0.097,0.675,0.271,1.325,0.526,1.935c-4.379,1.837-7.345,6.126-7.345,11.065c0,1.104,0.896,2,2,2s2-0.896,2-2c0-3.677,2.461-6.82,5.945-7.744c0.534,0.4,1.114,0.739,1.738,0.997c-1.798,4.396-0.863,9.526,2.63,13.019C39.977,60.805,40.488,61,41,61s1.023-0.195,1.414-0.585c0.781-0.781,0.781-2.047,0-2.829c-2.599-2.599-3.082-6.56-1.273-9.677c0.675-0.097,1.326-0.271,1.936-0.526c1.837,4.379,6.126,7.345,11.065,7.345c1.104,0,2-0.896,2-2s-0.896-2-2-2c-3.675,0-6.816-2.457-7.744-5.947c0.398-0.531,0.736-1.108,0.993-1.729c1.458,0.597,2.997,0.893,4.53,0.893c3.091,0,6.158-1.197,8.492-3.531c0.781-0.781,0.781-2.047,0-2.829c-0.781-0.78-2.047-0.781-2.828,0c-2.596,2.596-6.553,3.081-9.677,1.271c-0.097-0.675-0.271-1.325-0.526-1.935c4.379-1.837,7.345-6.126,7.345-11.065c0-1.104-0.896-2-2-2s-2,0.896-2,2c0,3.677-2.461,6.82-5.945,7.744c-0.534-0.4-1.114-0.739-1.738-0.997c1.798-4.396,0.863-9.526-2.63-13.019c-0.781-0.781-2.047-0.781-2.828,0c-0.781,0.781-0.781,2.047,0,2.829c2.599,2.599,3.082,6.56,1.273,9.677c-0.675,0.097-1.326,0.271-1.936,0.526c-1.837-4.379-6.126-7.345-11.065-7.345c-1.104,0-2,0.896-2,2s0.896,2,2,2c3.675,0,6.816,2.457,7.744,5.947c-0.4,0.533-0.739,1.114-0.997,1.738c-4.396-1.799-9.527-0.863-13.019,2.629c-0.781,0.781-0.781,2.047,0,2.829C19.977,42.805,20.488,43,21,43s1.023-0.195,1.414-0.586C25.012,39.817,28.967,39.333,32.091,41.143z M40,36c2.206,0,4,1.794,4,4s-1.794,4-4,4s-4-1.794-4-4S37.794,36,40,36z" />
        <circle cx="15" cy="15" r="2" fill="var(--waft-accent)" />
        <circle cx="65" cy="15" r="2" fill="var(--waft-accent)" />
        <circle cx="15" cy="65" r="2" fill="var(--waft-accent)" />
        <circle cx="65" cy="65" r="2" fill="var(--waft-accent)" />
      </g>
    </svg>
  );
}

const THEME_KEY = "waft-theme";
const THEME_CHOICES = [
  { value: "light", tip: "Light", icon: <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4M12 7.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9z" /> },
  { value: "dark", tip: "Dark", icon: <path d="M20 14.5A8 8 0 0 1 9.5 4a7 7 0 1 0 10.5 10.5z" /> },
  { value: "auto", tip: "System", icon: <g><rect x="2.5" y="4" width="19" height="12.5" rx="1.5" /><path d="M8.5 20.5h7M12 16.5v4" /></g> },
];

function readTheme() {
  try {
    const s = localStorage.getItem(THEME_KEY);
    if (s === "light" || s === "dark") return s;
  } catch (e) {}
  return "auto";
}

function ThemeToggle() {
  const [choice, setChoice] = React.useState(readTheme);
  const buttonRefs = React.useRef([]);
  React.useEffect(() => {
    document.documentElement.setAttribute("data-theme", choice);
    try { localStorage.setItem(THEME_KEY, choice); } catch (e) {}
  }, [choice]);
  function onKeyDown(event, index) {
    const keys = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"];
    if (!keys.includes(event.key)) return;
    event.preventDefault();
    const offset = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : -1;
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? THEME_CHOICES.length - 1
          : (index + offset + THEME_CHOICES.length) % THEME_CHOICES.length;
    setChoice(THEME_CHOICES[nextIndex].value);
    buttonRefs.current[nextIndex]?.focus();
  }
  return (
    <div className="theme-toggle" role="radiogroup" aria-label="Color theme">
      {THEME_CHOICES.map((c, index) => (
        <button
          key={c.value}
          ref={(element) => { buttonRefs.current[index] = element; }}
          type="button"
          className="theme-toggle__btn"
          role="radio"
          aria-checked={choice === c.value}
          aria-label={c.tip}
          title={c.tip}
          tabIndex={choice === c.value ? 0 : -1}
          onClick={() => setChoice(c.value)}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
            strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            {c.icon}
          </svg>
        </button>
      ))}
    </div>
  );
}

const NAV = [
  { label: "Overview", href: "#overview", screen: "landing" },
  { label: "Usage", href: "#usage", screen: "usage" },
  { label: "Include Format", href: "#worktreeinclude", screen: "worktreeinclude" },
  { label: "Safety", href: "#safety", screen: "safety" },
  { label: "Profiles", href: "#profiles", screen: "profiles" },
  { label: "GitHub", href: "https://github.com/plx/waft", external: true },
];

function Header({ screen, onNavigate }) {
  return (
    <header className="site-header">
      <div className="section-shell">
        <div className="site-nav">
          <a
            href="#overview"
            className="site-nav__brand"
            onClick={(e) => {
              e.preventDefault();
              onNavigate("landing");
            }}
          >
            <WaftMark />
            waft
          </a>
          <nav className="site-nav__links" aria-label="Primary">
            {NAV.filter((n) => !n.external).map((n) => (
              <a
                key={n.href}
                href={n.href}
                className={
                  "site-nav__link" +
                  (screen === n.screen ? " is-active" : "")
                }
                onClick={(e) => {
                  e.preventDefault();
                  onNavigate(n.screen);
                }}
              >
                {n.label}
              </a>
            ))}
          </nav>
          <div className="site-nav__actions">
            <ThemeToggle />
            <a
              href="https://github.com/plx/waft"
              className="button button--secondary"
            >
              GitHub
            </a>
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
          </div>
        </div>
      </div>
    </header>
  );
}

function Footer() {
  const links = [
    { label: "Usage", href: "#usage" },
    { label: "Include Format", href: "#worktreeinclude" },
    { label: "Safety Model", href: "#safety" },
    { label: "Architecture", href: "#architecture" },
    { label: "Configuration", href: "#configuration" },
    { label: "GitHub", href: "https://github.com/plx/waft" },
  ];
  return (
    <footer className="site-footer">
      <div className="section-shell">
        <div className="site-footer__inner">
          <p>waft · MIT · plx/waft</p>
          <nav aria-label="Footer">
            {links.map((l) => (
              <a key={l.href} href={l.href}>
                {l.label}
              </a>
            ))}
          </nav>
        </div>
      </div>
    </footer>
  );
}

Object.assign(window, { Header, Footer, WaftMark, ThemeToggle });
