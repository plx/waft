/* @ds-bundle: {"format":4,"namespace":"WaftDesignSystem_bb1143","components":[{"name":"Badge","sourcePath":"components/Badge.jsx"},{"name":"Button","sourcePath":"components/Button.jsx"},{"name":"TerminalCard","sourcePath":"components/TerminalCard.jsx"}],"sourceHashes":{"components/Badge.jsx":"e31ecb1f06db","components/Button.jsx":"0d700d33d393","components/TerminalCard.jsx":"7f92ea08ce25","site/src/scripts/landing.ts":"4d68bda2608d","ui_kits/site/Chrome.jsx":"9de05b95496b","ui_kits/site/DocsPage.jsx":"ad5ca294ef8c","ui_kits/site/Hero.jsx":"2b0b9f412948","ui_kits/site/Sections.jsx":"0ee34205b58e"},"inlinedExternals":[],"unexposedExports":[]} */

(() => {

const __ds_ns = (window.WaftDesignSystem_bb1143 = window.WaftDesignSystem_bb1143 || {});

const __ds_scope = {};

(__ds_ns.__errors = __ds_ns.__errors || []);

// components/Badge.jsx
try { (() => {
const React = typeof window !== "undefined" && window.React || globalThis.React;

/**
 * Waft badge / pill. Mono, 12px, hairline border.
 * tone: "neutral" (tags), "accent" (eligible), "warn" (excluded).
 */
function Badge({
  tone = "neutral",
  children,
  ...rest
}) {
  const base = {
    display: "inline-flex",
    alignItems: "center",
    minHeight: 28,
    padding: "0 9px",
    border: "1px solid var(--waft-line, #e2e5e9)",
    borderRadius: "var(--waft-radius-sm, 4px)",
    background: "var(--waft-panel, #fff)",
    color: "var(--waft-muted, #6b7280)",
    fontFamily: "var(--waft-font-mono, 'JetBrains Mono', monospace)",
    fontSize: "0.75rem",
    fontWeight: 600,
    whiteSpace: "nowrap"
  };
  const tones = {
    neutral: {},
    accent: {
      color: "var(--waft-accent-dark, #0f6e61)",
      borderColor: "color-mix(in srgb, var(--waft-accent, #16a085) 42%, transparent)",
      background: "var(--waft-accent-soft, #dff5f1)"
    },
    warn: {
      color: "var(--waft-error, #b42323)",
      borderColor: "color-mix(in srgb, var(--waft-error, #b42323) 34%, #e2e5e9)",
      background: "var(--waft-error-bg, #fce4e4)"
    }
  };
  const style = {
    ...base,
    ...(tones[tone] || tones.neutral)
  };
  return React.createElement("span", {
    className: "waft-badge",
    style,
    ...rest
  }, children);
}
Object.assign(__ds_scope, { Badge });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/Badge.jsx", error: String((e && e.message) || e) }); }

// components/Button.jsx
try { (() => {
const React = typeof window !== "undefined" && window.React || globalThis.React;

/**
 * Waft button. Three variants, all sharing the same 36px skeleton.
 * Mirrors .button / .button--primary / .button--secondary from the site.
 */
function Button({
  variant = "primary",
  children,
  href,
  onClick,
  type = "button",
  disabled = false,
  ...rest
}) {
  const base = {
    minHeight: 36,
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
    padding: "0 14px",
    border: "1px solid var(--waft-line, #e2e5e9)",
    borderRadius: "var(--waft-radius-sm, 4px)",
    background: "var(--waft-panel, #fff)",
    color: "var(--waft-ink, #1f2328)",
    fontFamily: "var(--waft-font-sans, Inter, system-ui, sans-serif)",
    fontWeight: 700,
    fontSize: "0.875rem",
    lineHeight: 1,
    whiteSpace: "nowrap",
    textDecoration: "none",
    boxShadow: "var(--waft-shadow, 0 1px 2px rgb(16 24 40 / 6%))",
    cursor: disabled ? "not-allowed" : "pointer",
    opacity: disabled ? 0.55 : 1
  };
  const variants = {
    primary: {
      background: "var(--waft-accent-dark, #0f6e61)",
      borderColor: "var(--waft-accent-dark, #0f6e61)",
      color: "var(--waft-surface, #f7f8f6)"
    },
    secondary: {
      background: "var(--waft-surface, #f7f8f6)"
    },
    ghost: {
      background: "transparent",
      boxShadow: "none"
    }
  };
  const style = {
    ...base,
    ...(variants[variant] || variants.primary)
  };
  const Tag = href ? "a" : "button";
  const tagProps = href ? {
    href
  } : {
    type
  };
  return React.createElement(Tag, {
    className: "waft-button",
    style,
    onClick,
    disabled: href ? undefined : disabled,
    ...tagProps,
    ...rest
  }, children);
}
Object.assign(__ds_scope, { Button });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/Button.jsx", error: String((e && e.message) || e) }); }

// components/TerminalCard.jsx
try { (() => {
const React = typeof window !== "undefined" && window.React || globalThis.React;

/**
 * Waft terminal card — the brand's hero element. Mono, header + Copy.
 * Theme-aware: light surface in light mode, dark surface in dark mode.
 * `lines` is an array of command lines; `copyText` overrides what Copy writes.
 */
function TerminalCard({
  title = "waft",
  meta = "copy commands",
  lines = [],
  copyText,
  ...rest
}) {
  const [copied, setCopied] = React.useState(false);
  const onCopy = () => {
    try {
      const text = copyText || lines.join("\n");
      navigator.clipboard && navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch (e) {}
  };
  const hairline = "1px solid var(--waft-line, #e2e5e9)";
  const dim = "var(--waft-ink-soft, #3d4650)";
  const fg = "var(--waft-code-fg, #1f2328)";
  return React.createElement("div", {
    className: "waft-terminal",
    style: {
      overflow: "hidden",
      border: hairline,
      borderRadius: "var(--waft-radius, 8px)",
      background: "var(--waft-code-bg, #f3f5f4)",
      color: fg,
      boxShadow: "var(--waft-shadow, 0 1px 2px rgb(16 24 40 / 6%))",
      fontFamily: "var(--waft-font-mono, 'JetBrains Mono', monospace)",
      fontSize: "0.875rem",
      lineHeight: 1.55
    },
    ...rest
  }, React.createElement("div", {
    style: {
      minHeight: 40,
      display: "flex",
      alignItems: "center",
      justifyContent: "space-between",
      gap: 16,
      padding: "0 14px",
      borderBottom: hairline,
      color: dim,
      fontFamily: "var(--waft-font-sans, Inter, system-ui, sans-serif)"
    }
  }, React.createElement("div", null, React.createElement("p", {
    style: {
      margin: "0 0 2px",
      color: dim,
      fontSize: "0.7rem",
      fontWeight: 800,
      textTransform: "uppercase"
    }
  }, "Terminal"), React.createElement("h2", {
    style: {
      margin: 0,
      color: fg,
      fontSize: "0.95rem",
      fontWeight: 700
    }
  }, title)), React.createElement("span", {
    style: {
      color: dim,
      fontSize: "0.75rem"
    }
  }, meta)), React.createElement("pre", {
    style: {
      margin: 0,
      padding: 14,
      overflowX: "auto",
      whiteSpace: "pre-wrap",
      color: fg
    }
  }, React.createElement("code", null, lines.join("\n"))), React.createElement("button", {
    type: "button",
    onClick: onCopy,
    style: {
      display: "inline-flex",
      alignItems: "center",
      gap: 6,
      minHeight: 30,
      margin: "0 0 14px 14px",
      padding: "0 10px",
      border: hairline,
      borderRadius: "var(--waft-radius-sm, 4px)",
      background: "var(--waft-code-btn, #ffffff)",
      color: fg,
      fontFamily: "var(--waft-font-mono, 'JetBrains Mono', monospace)",
      fontSize: "0.75rem",
      cursor: "pointer"
    }
  }, copied ? "Copied" : "Copy"));
}
Object.assign(__ds_scope, { TerminalCard });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/TerminalCard.jsx", error: String((e && e.message) || e) }); }

// site/src/scripts/landing.ts
try { (() => {
const navToggle = document.querySelector("[data-nav-toggle]");
const navPanel = document.querySelector("[data-nav-panel]");
const navLinks = document.querySelectorAll("[data-nav-link]");
const THEME_KEY = "waft-theme";
function readTheme() {
  try {
    const stored = localStorage.getItem(THEME_KEY);
    if (stored === "light" || stored === "dark") {
      return stored;
    }
  } catch {
    // localStorage unavailable — fall through to auto.
  }
  return "auto";
}
const themeButtons = document.querySelectorAll("[data-theme-choice]");
function applyTheme(choice) {
  document.documentElement.setAttribute("data-theme", choice);
  themeButtons.forEach(button => {
    button.setAttribute("aria-checked", String(button.dataset.themeChoice === choice));
  });
}
applyTheme(readTheme());
themeButtons.forEach(button => {
  button.addEventListener("click", () => {
    const choice = button.dataset.themeChoice;
    if (choice !== "light" && choice !== "dark" && choice !== "auto") {
      return;
    }
    try {
      localStorage.setItem(THEME_KEY, choice);
    } catch {
      // Persisting is best-effort.
    }
    applyTheme(choice);
  });
});
function setNavOpen(open) {
  if (!navToggle || !navPanel) {
    return;
  }
  navToggle.setAttribute("aria-expanded", String(open));
  navToggle.setAttribute("aria-label", open ? "Close navigation" : "Open navigation");
  navPanel.hidden = !open;
  navPanel.dataset.open = String(open);
}
navToggle?.addEventListener("click", () => {
  setNavOpen(navToggle.getAttribute("aria-expanded") !== "true");
});
navLinks.forEach(link => {
  link.addEventListener("click", () => setNavOpen(false));
});
document.addEventListener("keydown", event => {
  if (event.key === "Escape") {
    setNavOpen(false);
  }
});
async function copyText(text) {
  if (!navigator.clipboard) {
    throw new Error("Clipboard API is unavailable.");
  }
  await navigator.clipboard.writeText(text);
}
document.querySelectorAll("[data-copy-text]").forEach(button => {
  let resetTimer;
  const visibleLabel = button.querySelector("span:not(.sr-only)");
  const status = button.querySelector("[data-copy-status]");
  const defaultVisibleText = visibleLabel?.textContent || "Copy";
  button.addEventListener("click", async () => {
    const text = button.dataset.copyText;
    if (!text) {
      return;
    }
    window.clearTimeout(resetTimer);
    try {
      await copyText(text);
      if (visibleLabel) {
        visibleLabel.textContent = "Copied";
      }
      if (status) {
        status.textContent = "Command copied to clipboard.";
      }
    } catch {
      if (visibleLabel) {
        visibleLabel.textContent = "Copy failed";
      }
      if (status) {
        status.textContent = "Copy failed.";
      }
    }
    resetTimer = window.setTimeout(() => {
      if (visibleLabel) {
        visibleLabel.textContent = defaultVisibleText;
      }
      if (status) {
        status.textContent = "";
      }
    }, 2200);
  });
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "site/src/scripts/landing.ts", error: String((e && e.message) || e) }); }

// ui_kits/site/Chrome.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
// Site shared layout primitives — Header (sticky nav) + Footer.
// Both are fully theme-aware (light in light mode, dark in dark mode).
// Used by every screen in the kit.

// Chip-less, theme-aware brand mark: boxed computer-fan line art —
// teal frame + rim, mustard blades. Colors from --waft-accent / --waft-mark-2
// so it reads on any surface.
function WaftMark(props) {
  return /*#__PURE__*/React.createElement("svg", _extends({
    viewBox: "0 0 80 80",
    role: "img",
    "aria-label": "waft mark"
  }, props), /*#__PURE__*/React.createElement("g", {
    transform: "translate(40,40) scale(0.82) translate(-40,-40)"
  }, /*#__PURE__*/React.createElement("path", {
    fill: "var(--waft-accent)",
    d: "M7,67c0,3.309,2.691,6,6,6h54c3.309,0,6-2.691,6-6V13c0-3.309-2.691-6-6-6H13c-3.309,0-6,2.691-6,6V67z M11,13c0-1.103,0.897-2,2-2h54c1.103,0,2,0.897,2,2v54c0,1.103-0.897,2-2,2H13c-1.103,0-2-0.897-2-2V13z"
  }), /*#__PURE__*/React.createElement("path", {
    fill: "var(--waft-accent)",
    d: "M40,67c14.888,0,27-12.112,27-27S54.888,13,40,13S13,25.112,13,40S25.112,67,40,67z M40,17c12.683,0,23,10.318,23,23S52.683,63,40,63S17,52.682,17,40S27.317,17,40,17z"
  }), /*#__PURE__*/React.createElement("path", {
    fill: "var(--waft-mark-2)",
    d: "M32.091,41.143c0.097,0.675,0.271,1.325,0.526,1.935c-4.379,1.837-7.345,6.126-7.345,11.065c0,1.104,0.896,2,2,2s2-0.896,2-2c0-3.677,2.461-6.82,5.945-7.744c0.534,0.4,1.114,0.739,1.738,0.997c-1.798,4.396-0.863,9.526,2.63,13.019C39.977,60.805,40.488,61,41,61s1.023-0.195,1.414-0.585c0.781-0.781,0.781-2.047,0-2.829c-2.599-2.599-3.082-6.56-1.273-9.677c0.675-0.097,1.326-0.271,1.936-0.526c1.837,4.379,6.126,7.345,11.065,7.345c1.104,0,2-0.896,2-2s-0.896-2-2-2c-3.675,0-6.816-2.457-7.744-5.947c0.398-0.531,0.736-1.108,0.993-1.729c1.458,0.597,2.997,0.893,4.53,0.893c3.091,0,6.158-1.197,8.492-3.531c0.781-0.781,0.781-2.047,0-2.829c-0.781-0.78-2.047-0.781-2.828,0c-2.596,2.596-6.553,3.081-9.677,1.271c-0.097-0.675-0.271-1.325-0.526-1.935c4.379-1.837,7.345-6.126,7.345-11.065c0-1.104-0.896-2-2-2s-2,0.896-2,2c0,3.677-2.461,6.82-5.945,7.744c-0.534-0.4-1.114-0.739-1.738-0.997c1.798-4.396,0.863-9.526-2.63-13.019c-0.781-0.781-2.047-0.781-2.828,0c-0.781,0.781-0.781,2.047,0,2.829c2.599,2.599,3.082,6.56,1.273,9.677c-0.675,0.097-1.326,0.271-1.936,0.526c-1.837-4.379-6.126-7.345-11.065-7.345c-1.104,0-2,0.896-2,2s0.896,2,2,2c3.675,0,6.816,2.457,7.744,5.947c-0.4,0.533-0.739,1.114-0.997,1.738c-4.396-1.799-9.527-0.863-13.019,2.629c-0.781,0.781-0.781,2.047,0,2.829C19.977,42.805,20.488,43,21,43s1.023-0.195,1.414-0.586C25.012,39.817,28.967,39.333,32.091,41.143z M40,36c2.206,0,4,1.794,4,4s-1.794,4-4,4s-4-1.794-4-4S37.794,36,40,36z"
  }), /*#__PURE__*/React.createElement("circle", {
    cx: "15",
    cy: "15",
    r: "2",
    fill: "var(--waft-accent)"
  }), /*#__PURE__*/React.createElement("circle", {
    cx: "65",
    cy: "15",
    r: "2",
    fill: "var(--waft-accent)"
  }), /*#__PURE__*/React.createElement("circle", {
    cx: "15",
    cy: "65",
    r: "2",
    fill: "var(--waft-accent)"
  }), /*#__PURE__*/React.createElement("circle", {
    cx: "65",
    cy: "65",
    r: "2",
    fill: "var(--waft-accent)"
  })));
}
const THEME_KEY = "waft-theme";
const THEME_CHOICES = [{
  value: "light",
  tip: "Light",
  icon: /*#__PURE__*/React.createElement("path", {
    d: "M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4M12 7.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9z"
  })
}, {
  value: "dark",
  tip: "Dark",
  icon: /*#__PURE__*/React.createElement("path", {
    d: "M20 14.5A8 8 0 0 1 9.5 4a7 7 0 1 0 10.5 10.5z"
  })
}, {
  value: "auto",
  tip: "System",
  icon: /*#__PURE__*/React.createElement("g", null, /*#__PURE__*/React.createElement("rect", {
    x: "2.5",
    y: "4",
    width: "19",
    height: "12.5",
    rx: "1.5"
  }), /*#__PURE__*/React.createElement("path", {
    d: "M8.5 20.5h7M12 16.5v4"
  }))
}];
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
    try {
      localStorage.setItem(THEME_KEY, choice);
    } catch (e) {}
  }, [choice]);
  function onKeyDown(event, index) {
    const keys = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"];
    if (!keys.includes(event.key)) return;
    event.preventDefault();
    const offset = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : -1;
    const nextIndex = event.key === "Home" ? 0 : event.key === "End" ? THEME_CHOICES.length - 1 : (index + offset + THEME_CHOICES.length) % THEME_CHOICES.length;
    setChoice(THEME_CHOICES[nextIndex].value);
    buttonRefs.current[nextIndex]?.focus();
  }
  return /*#__PURE__*/React.createElement("div", {
    className: "theme-toggle",
    role: "radiogroup",
    "aria-label": "Color theme"
  }, THEME_CHOICES.map((c, index) => /*#__PURE__*/React.createElement("button", {
    key: c.value,
    ref: element => {
      buttonRefs.current[index] = element;
    },
    type: "button",
    className: "theme-toggle__btn",
    role: "radio",
    "aria-checked": choice === c.value,
    "aria-label": c.tip,
    title: c.tip,
    tabIndex: choice === c.value ? 0 : -1,
    onClick: () => setChoice(c.value),
    onKeyDown: event => onKeyDown(event, index)
  }, /*#__PURE__*/React.createElement("svg", {
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: "2",
    strokeLinecap: "round",
    strokeLinejoin: "round",
    "aria-hidden": "true"
  }, c.icon))));
}
const NAV = [{
  label: "Overview",
  href: "#overview",
  screen: "landing"
}, {
  label: "Usage",
  href: "#usage",
  screen: "usage"
}, {
  label: "Include Format",
  href: "#worktreeinclude",
  screen: "worktreeinclude"
}, {
  label: "Safety",
  href: "#safety",
  screen: "safety"
}, {
  label: "Profiles",
  href: "#profiles",
  screen: "profiles"
}, {
  label: "GitHub",
  href: "https://github.com/plx/waft",
  external: true
}];
function Header({
  screen,
  onNavigate
}) {
  return /*#__PURE__*/React.createElement("header", {
    className: "site-header"
  }, /*#__PURE__*/React.createElement("div", {
    className: "section-shell"
  }, /*#__PURE__*/React.createElement("div", {
    className: "site-nav"
  }, /*#__PURE__*/React.createElement("a", {
    href: "#overview",
    className: "site-nav__brand",
    onClick: e => {
      e.preventDefault();
      onNavigate("landing");
    }
  }, /*#__PURE__*/React.createElement(WaftMark, null), "waft"), /*#__PURE__*/React.createElement("nav", {
    className: "site-nav__links",
    "aria-label": "Primary"
  }, NAV.filter(n => !n.external).map(n => /*#__PURE__*/React.createElement("a", {
    key: n.href,
    href: n.href,
    className: "site-nav__link" + (screen === n.screen ? " is-active" : ""),
    onClick: e => {
      e.preventDefault();
      onNavigate(n.screen);
    }
  }, n.label))), /*#__PURE__*/React.createElement("div", {
    className: "site-nav__actions"
  }, /*#__PURE__*/React.createElement(ThemeToggle, null), /*#__PURE__*/React.createElement("a", {
    href: "https://github.com/plx/waft",
    className: "button button--secondary"
  }, "GitHub"), /*#__PURE__*/React.createElement("a", {
    href: "#usage",
    className: "button button--primary",
    onClick: e => {
      e.preventDefault();
      onNavigate("usage");
    }
  }, "Read the docs")))));
}
function Footer() {
  const links = [{
    label: "Usage",
    href: "#usage"
  }, {
    label: "Include Format",
    href: "#worktreeinclude"
  }, {
    label: "Safety Model",
    href: "#safety"
  }, {
    label: "Architecture",
    href: "#architecture"
  }, {
    label: "Configuration",
    href: "#configuration"
  }, {
    label: "GitHub",
    href: "https://github.com/plx/waft"
  }];
  return /*#__PURE__*/React.createElement("footer", {
    className: "site-footer"
  }, /*#__PURE__*/React.createElement("div", {
    className: "section-shell"
  }, /*#__PURE__*/React.createElement("div", {
    className: "site-footer__inner"
  }, /*#__PURE__*/React.createElement("p", null, "waft \xB7 MIT \xB7 plx/waft"), /*#__PURE__*/React.createElement("nav", {
    "aria-label": "Footer"
  }, links.map(l => /*#__PURE__*/React.createElement("a", {
    key: l.href,
    href: l.href
  }, l.label))))));
}
Object.assign(window, {
  Header,
  Footer,
  WaftMark,
  ThemeToggle
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/site/Chrome.jsx", error: String((e && e.message) || e) }); }

// ui_kits/site/DocsPage.jsx
try { (() => {
// Docs page — Starlight-style sidebar + main column.
// Renders the actual MDX docs as JSX (content drawn from site/src/content/docs/*.mdx).

const DOC_PAGES = [{
  slug: "usage",
  title: "Usage",
  lede: "Copy ignored files selected by .worktreeinclude between Git worktrees."
}, {
  slug: "worktreeinclude",
  title: ".worktreeinclude",
  lede: "The include file format used by waft."
}, {
  slug: "safety",
  title: "Safety",
  lede: "The guarantees waft keeps while copying ignored worktree files."
}, {
  slug: "profiles",
  title: "Profiles",
  lede: "Compatibility profiles for different worktree workflows."
}, {
  slug: "configuration",
  title: "Configuration",
  lede: "Layered configuration and per-knob overrides."
}, {
  slug: "architecture",
  title: "Architecture",
  lede: "How waft plans and executes safe worktree file copies."
}];
function Sidebar({
  active,
  onNavigate
}) {
  return /*#__PURE__*/React.createElement("aside", {
    className: "docs__sidebar",
    "aria-label": "Docs"
  }, /*#__PURE__*/React.createElement("p", {
    className: "docs__sidebar-heading"
  }, "Guides"), /*#__PURE__*/React.createElement("ul", null, DOC_PAGES.map(p => /*#__PURE__*/React.createElement("li", {
    key: p.slug
  }, /*#__PURE__*/React.createElement("a", {
    href: "#" + p.slug,
    className: active === p.slug ? "is-active" : "",
    onClick: e => {
      e.preventDefault();
      onNavigate(p.slug);
    }
  }, p.title)))));
}
function CodeBlock({
  children,
  lang
}) {
  return /*#__PURE__*/React.createElement("pre", null, /*#__PURE__*/React.createElement("code", {
    className: lang ? "lang-" + lang : ""
  }, children));
}

// ── Page bodies ─────────────────────────────────────────────────────────────

function UsagePage() {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("h1", null, "Usage"), /*#__PURE__*/React.createElement("p", {
    className: "lede"
  }, "Copy ignored files selected by ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), " between Git worktrees."), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "waft"), " copies ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), "-selected ignored files between Git worktrees. It is intended for local configuration, caches, and tool state that Git should not track but that developers often need in more than one linked worktree."), /*#__PURE__*/React.createElement("h2", null, "Quick start"), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `# In a linked worktree, copy from the main worktree automatically.
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
waft validate`), /*#__PURE__*/React.createElement("h2", null, "Commands"), /*#__PURE__*/React.createElement("table", null, /*#__PURE__*/React.createElement("thead", null, /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("th", null, "Command"), /*#__PURE__*/React.createElement("th", null, "Description"))), /*#__PURE__*/React.createElement("tbody", null, /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "waft"), " / ", /*#__PURE__*/React.createElement("code", null, "waft copy")), /*#__PURE__*/React.createElement("td", null, "Copy eligible files. This is the default command.")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "waft list")), /*#__PURE__*/React.createElement("td", null, "List eligible files without copying.")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "waft info <PATH>...")), /*#__PURE__*/React.createElement("td", null, "Show detailed status for specific files.")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "waft validate")), /*#__PURE__*/React.createElement("td", null, "Check ignore files for syntax errors.")))), /*#__PURE__*/React.createElement("h2", null, "Global options"), /*#__PURE__*/React.createElement("table", null, /*#__PURE__*/React.createElement("thead", null, /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("th", null, "Option"), /*#__PURE__*/React.createElement("th", null, "Description"))), /*#__PURE__*/React.createElement("tbody", null, /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "--source <PATH>")), /*#__PURE__*/React.createElement("td", null, "Source, usually the main worktree path.")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "--dest <PATH>")), /*#__PURE__*/React.createElement("td", null, "Destination, usually the linked worktree path.")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "-C <PATH>")), /*#__PURE__*/React.createElement("td", null, "Operate as if started in ", /*#__PURE__*/React.createElement("code", null, "PATH"), ".")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "-q, --quiet")), /*#__PURE__*/React.createElement("td", null, "Suppress non-error output.")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "-v, --verbose")), /*#__PURE__*/React.createElement("td", null, "Increase output verbosity.")))), /*#__PURE__*/React.createElement("h2", null, "Typical flow"), /*#__PURE__*/React.createElement("ol", null, /*#__PURE__*/React.createElement("li", null, "Add ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), " to the source repo."), /*#__PURE__*/React.createElement("li", null, "Run ", /*#__PURE__*/React.createElement("code", null, "waft copy --dry-run"), "."), /*#__PURE__*/React.createElement("li", null, "Inspect the plan."), /*#__PURE__*/React.createElement("li", null, "Run ", /*#__PURE__*/React.createElement("code", null, "waft copy"), "."), /*#__PURE__*/React.createElement("li", null, "Use ", /*#__PURE__*/React.createElement("code", null, "waft info <path>"), " when a file is missing or skipped.")));
}
function WorktreeIncludePage() {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("h1", null, ".worktreeinclude"), /*#__PURE__*/React.createElement("p", {
    className: "lede"
  }, "The include file format used by waft."), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), " uses familiar ", /*#__PURE__*/React.createElement("code", null, ".gitignore"), " syntax to select ignored files that ", /*#__PURE__*/React.createElement("code", null, "waft"), " should copy between worktrees."), /*#__PURE__*/React.createElement(CodeBlock, null, `# Include environment files
.env
*.env.local

# Include all secret keys recursively
**/*.key

# But not test keys
!test.key`), /*#__PURE__*/React.createElement("h2", null, "Eligibility"), /*#__PURE__*/React.createElement("p", null, "A file is eligible for copying only when all of these are true:"), /*#__PURE__*/React.createElement("ol", null, /*#__PURE__*/React.createElement("li", null, "It exists in the source worktree."), /*#__PURE__*/React.createElement("li", null, "It is a regular file, not a symlink, directory, or special file."), /*#__PURE__*/React.createElement("li", null, "It is selected by the active compatibility profile."), /*#__PURE__*/React.createElement("li", null, "Git confirms it is ignored and untracked."), /*#__PURE__*/React.createElement("li", null, "It is not dropped by the active exclusion set.")), /*#__PURE__*/React.createElement("h2", null, "Profile-dependent matching"), /*#__PURE__*/React.createElement("p", null, "By default, the ", /*#__PURE__*/React.createElement("code", null, "claude"), " profile only consults the root-level ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), "."), /*#__PURE__*/React.createElement("p", null, "Use ", /*#__PURE__*/React.createElement("code", null, "--compat-profile git"), " when you want nested ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), " files to compose like nested ", /*#__PURE__*/React.createElement("code", null, ".gitignore"), " files. Use ", /*#__PURE__*/React.createElement("code", null, "--compat-profile wt"), " when you want worktrunk-compatible all-ignored behavior."), /*#__PURE__*/React.createElement("h2", null, "Validation"), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `waft validate`), /*#__PURE__*/React.createElement("p", null, "Validation parses ignore files before copy execution. Copy planning remains read-only until the command enters the execute phase."));
}
function SafetyPage() {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("h1", null, "Safety"), /*#__PURE__*/React.createElement("p", {
    className: "lede"
  }, "The guarantees waft keeps while copying ignored worktree files."), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "waft"), " is designed around a plan-then-execute pipeline. Discovery and classification happen before any filesystem mutation."), /*#__PURE__*/React.createElement("h2", null, "Guarantees"), /*#__PURE__*/React.createElement("ul", null, /*#__PURE__*/React.createElement("li", null, /*#__PURE__*/React.createElement("strong", null, "Tracked files are never overwritten."), " Destination trackedness is checked before copy execution."), /*#__PURE__*/React.createElement("li", null, /*#__PURE__*/React.createElement("strong", null, "Symlink traversal is blocked."), " ", /*#__PURE__*/React.createElement("code", null, "waft"), " refuses to follow symlinks in source files or write through symlinked destination parents."), /*#__PURE__*/React.createElement("li", null, /*#__PURE__*/React.createElement("strong", null, "Atomic writes."), " Files are written to a temporary path and then renamed into place."), /*#__PURE__*/React.createElement("li", null, /*#__PURE__*/React.createElement("strong", null, "Dry runs are mutation-free."), " ", /*#__PURE__*/React.createElement("code", null, "waft copy --dry-run"), " reads and reports only.")), /*#__PURE__*/React.createElement("h2", null, "Preview before writing"), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `waft copy --dry-run`), /*#__PURE__*/React.createElement("p", null, "Use dry-run output to confirm which files would move and why."), /*#__PURE__*/React.createElement("h2", null, "Overwrite behavior"), /*#__PURE__*/React.createElement("p", null, "Existing destination files are preserved by default. Use ", /*#__PURE__*/React.createElement("code", null, "--overwrite"), " only when you intentionally want untracked destination files replaced."), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `waft copy --overwrite`), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "--overwrite"), " does not change the rule that tracked files are protected."));
}
function ProfilesPage() {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("h1", null, "Profiles"), /*#__PURE__*/React.createElement("p", {
    className: "lede"
  }, "Compatibility profiles for different worktree workflows."), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "waft"), " ships with coordinated compatibility profiles. Each profile selects a bundle of matcher and safety behavior."), /*#__PURE__*/React.createElement("table", null, /*#__PURE__*/React.createElement("thead", null, /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("th", null, "Profile"), /*#__PURE__*/React.createElement("th", null, "Missing ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude")), /*#__PURE__*/React.createElement("th", null, "Matcher semantics"), /*#__PURE__*/React.createElement("th", null, "Symlinked rule files"), /*#__PURE__*/React.createElement("th", null, "Tool-state excludes"))), /*#__PURE__*/React.createElement("tbody", null, /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "claude")), /*#__PURE__*/React.createElement("td", null, "nothing selected"), /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "claude-2026-04")), /*#__PURE__*/React.createElement("td", null, "follow"), /*#__PURE__*/React.createElement("td", null, "none")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "git")), /*#__PURE__*/React.createElement("td", null, "nothing selected"), /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "git")), /*#__PURE__*/React.createElement("td", null, "ignore"), /*#__PURE__*/React.createElement("td", null, "none")), /*#__PURE__*/React.createElement("tr", null, /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "wt")), /*#__PURE__*/React.createElement("td", null, "every git-ignored untracked file selected"), /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "wt-0.39")), /*#__PURE__*/React.createElement("td", null, "follow"), /*#__PURE__*/React.createElement("td", null, /*#__PURE__*/React.createElement("code", null, "tooling-v1"))))), /*#__PURE__*/React.createElement("h2", null, /*#__PURE__*/React.createElement("code", null, "claude")), /*#__PURE__*/React.createElement("p", null, "The default profile. It matches Claude Code's out-of-the-box behavior by using the repository root ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), "."), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `waft copy --compat-profile claude`), /*#__PURE__*/React.createElement("h2", null, /*#__PURE__*/React.createElement("code", null, "git")), /*#__PURE__*/React.createElement("p", null, "Use this profile when you want Git-style per-directory exclude semantics, where patterns are relative to the directory containing the file and deeper files can override shallower files."), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `waft copy --compat-profile git`), /*#__PURE__*/React.createElement("h2", null, /*#__PURE__*/React.createElement("code", null, "wt")), /*#__PURE__*/React.createElement("p", null, "Use this profile for worktrunk parity. It starts from every git-ignored untracked file and removes paths matched by literal-name negations."), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "sh"
  }, `waft copy --compat-profile wt`));
}
function ConfigurationPage() {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("h1", null, "Configuration"), /*#__PURE__*/React.createElement("p", {
    className: "lede"
  }, "Layered configuration and per-knob overrides."), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "waft"), " resolves profile and policy settings from multiple layers. Later layers win for scalar values."), /*#__PURE__*/React.createElement("ol", null, /*#__PURE__*/React.createElement("li", null, "Built-in defaults."), /*#__PURE__*/React.createElement("li", null, "User config: ", /*#__PURE__*/React.createElement("code", null, "~/.config/waft/config.toml"), "."), /*#__PURE__*/React.createElement("li", null, "Project configs: each ", /*#__PURE__*/React.createElement("code", null, ".waft.toml"), " from repo root down to the current directory."), /*#__PURE__*/React.createElement("li", null, "Environment variables using the ", /*#__PURE__*/React.createElement("code", null, "WAFT_*"), " prefix."), /*#__PURE__*/React.createElement("li", null, "CLI flags.")), /*#__PURE__*/React.createElement("p", null, "Explicit knob settings always beat preset values from a higher-precedence profile layer."), /*#__PURE__*/React.createElement("h2", null, "Example ", /*#__PURE__*/React.createElement("code", null, ".waft.toml")), /*#__PURE__*/React.createElement(CodeBlock, {
    lang: "toml"
  }, `version = 1

[compat]
profile = "git"

[exclude]
extra = ["*.bak"]`));
}
function ArchitecturePage() {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("h1", null, "Architecture"), /*#__PURE__*/React.createElement("p", {
    className: "lede"
  }, "How waft plans and executes safe worktree file copies."), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "waft"), " is built around a few invariant design decisions."), /*#__PURE__*/React.createElement("h2", null, "Git is authoritative"), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, "waft"), " does not reimplement Git's ignore logic for final ignored/tracked decisions. Those checks go through the ", /*#__PURE__*/React.createElement("code", null, "GitBackend"), " trait."), /*#__PURE__*/React.createElement("ul", null, /*#__PURE__*/React.createElement("li", null, /*#__PURE__*/React.createElement("code", null, "GitGix"), " is the default in-process backend built on the ", /*#__PURE__*/React.createElement("code", null, "gix"), " crate."), /*#__PURE__*/React.createElement("li", null, /*#__PURE__*/React.createElement("code", null, "GitCli"), " shells out to Git and is selected with ", /*#__PURE__*/React.createElement("code", null, "WAFT_GIT_BACKEND=cli"), ".")), /*#__PURE__*/React.createElement("h2", null, "Commands plan before executing"), /*#__PURE__*/React.createElement("p", null, "All commands follow a read-only planning pipeline before writes are possible."), /*#__PURE__*/React.createElement("ol", null, /*#__PURE__*/React.createElement("li", null, "Parse CLI arguments."), /*#__PURE__*/React.createElement("li", null, "Resolve layered policy."), /*#__PURE__*/React.createElement("li", null, "Resolve source and destination worktree context."), /*#__PURE__*/React.createElement("li", null, "Validate ignore files."), /*#__PURE__*/React.createElement("li", null, "Select candidate files."), /*#__PURE__*/React.createElement("li", null, "Apply policy filters."), /*#__PURE__*/React.createElement("li", null, "Confirm ignored status through Git."), /*#__PURE__*/React.createElement("li", null, "Build a copy plan."), /*#__PURE__*/React.createElement("li", null, "Execute only for ", /*#__PURE__*/React.createElement("code", null, "copy"), " without ", /*#__PURE__*/React.createElement("code", null, "--dry-run"), ".")));
}
const PAGE_BY_SLUG = {
  usage: UsagePage,
  worktreeinclude: WorktreeIncludePage,
  safety: SafetyPage,
  profiles: ProfilesPage,
  configuration: ConfigurationPage,
  architecture: ArchitecturePage
};
function DocsPage({
  slug,
  onNavigate
}) {
  const Page = PAGE_BY_SLUG[slug] || UsagePage;
  return /*#__PURE__*/React.createElement("div", {
    className: "section-shell docs"
  }, /*#__PURE__*/React.createElement(Sidebar, {
    active: slug,
    onNavigate: onNavigate
  }), /*#__PURE__*/React.createElement("main", {
    className: "docs__main",
    "data-screen-label": `docs/${slug}`
  }, /*#__PURE__*/React.createElement(Page, null)));
}
Object.assign(window, {
  DocsPage,
  DOC_PAGES
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/site/DocsPage.jsx", error: String((e && e.message) || e) }); }

// ui_kits/site/Hero.jsx
try { (() => {
// Hero — landing-page first fold. Two-column on wide, stacked on narrow.

function TerminalCard({
  title = "waft",
  meta = "copy commands",
  lines,
  copyText
}) {
  const [copied, setCopied] = React.useState(false);
  const onCopy = () => {
    try {
      navigator.clipboard?.writeText(copyText || lines.join("\n"));
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch (_) {}
  };
  return /*#__PURE__*/React.createElement("div", {
    className: "command-card"
  }, /*#__PURE__*/React.createElement("div", {
    className: "command-card__header"
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement("p", {
    className: "eyebrow"
  }, "Terminal"), /*#__PURE__*/React.createElement("h2", null, title)), /*#__PURE__*/React.createElement("span", null, meta)), /*#__PURE__*/React.createElement("pre", {
    className: "command-card__body"
  }, /*#__PURE__*/React.createElement("code", null, lines.join("\n"))), /*#__PURE__*/React.createElement("button", {
    className: "copy-button",
    onClick: onCopy,
    type: "button"
  }, copied ? "Copied" : "Copy"));
}
function TransferPanel() {
  const includes = [".env", "*.env.local", "**/*.key"];
  return /*#__PURE__*/React.createElement("aside", {
    className: "transfer-panel"
  }, /*#__PURE__*/React.createElement("h2", null, "What gets copied"), /*#__PURE__*/React.createElement("p", null, /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), " selects files using familiar gitignore syntax."), /*#__PURE__*/React.createElement("div", {
    className: "transfer-list"
  }, includes.map(s => /*#__PURE__*/React.createElement("span", {
    key: s
  }, s))), /*#__PURE__*/React.createElement("div", {
    className: "transfer-rule"
  }, /*#__PURE__*/React.createElement("strong", null, "! test.key"), /*#__PURE__*/React.createElement("span", null, "Negations remove a path from the include set.")));
}
function Hero({
  onNavigate
}) {
  return /*#__PURE__*/React.createElement("section", {
    className: "section-shell hero",
    id: "overview"
  }, /*#__PURE__*/React.createElement("div", {
    className: "hero__copy"
  }, /*#__PURE__*/React.createElement("p", {
    className: "eyebrow"
  }, "Worktree-aware file tool"), /*#__PURE__*/React.createElement("h1", null, "Plan before copying."), /*#__PURE__*/React.createElement("p", {
    className: "hero__lede"
  }, /*#__PURE__*/React.createElement("code", null, "waft"), " copies ", /*#__PURE__*/React.createElement("code", null, ".worktreeinclude"), "-selected ignored files between Git worktrees."), /*#__PURE__*/React.createElement("p", {
    className: "hero__body"
  }, "A small Rust CLI for copying selected ignored files \u2014 env files, API keys, build caches \u2014 between linked worktrees, with a plan-then-execute safety model."), /*#__PURE__*/React.createElement("div", {
    className: "badge-row"
  }, /*#__PURE__*/React.createElement("span", {
    className: "badge"
  }, "Rust"), /*#__PURE__*/React.createElement("span", {
    className: "badge"
  }, "CLI"), /*#__PURE__*/React.createElement("span", {
    className: "badge"
  }, "Git worktrees")), /*#__PURE__*/React.createElement("div", {
    className: "hero__actions"
  }, /*#__PURE__*/React.createElement("a", {
    href: "#usage",
    className: "button button--primary",
    onClick: e => {
      e.preventDefault();
      onNavigate("usage");
    }
  }, "Read the docs"), /*#__PURE__*/React.createElement("a", {
    href: "https://github.com/plx/waft",
    className: "button button--secondary"
  }, "View on GitHub"))), /*#__PURE__*/React.createElement("div", {
    className: "hero__visual"
  }, /*#__PURE__*/React.createElement(TerminalCard, {
    lines: ["$ waft copy --dry-run", "$ waft copy --source ../main --dest ."],
    copyText: "waft copy --dry-run\nwaft copy --source ../main --dest ."
  }), /*#__PURE__*/React.createElement(TransferPanel, null)));
}
Object.assign(window, {
  Hero,
  TerminalCard,
  TransferPanel
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/site/Hero.jsx", error: String((e && e.message) || e) }); }

// ui_kits/site/Sections.jsx
try { (() => {
// Feature grid + Docs preview grid — landing page mid-sections.

function FeatureGrid({
  onNavigate
}) {
  const features = [{
    idx: "01",
    eyebrow: "Quick start",
    title: "Plan before copying",
    body: "Add .worktreeinclude, run waft copy --dry-run, inspect the plan, then run waft copy.",
    cta: "Read usage",
    screen: "usage"
  }, {
    idx: "02",
    eyebrow: "Include format",
    title: "Select ignored files with Gitignore-style patterns",
    body: ".worktreeinclude chooses the ignored, untracked regular files that should move between worktrees.",
    cta: "Read format",
    screen: "worktreeinclude"
  }, {
    idx: "03",
    eyebrow: "Safety",
    title: "Tracked files and symlinks stay protected",
    body: "waft refuses tracked overwrites, source symlinks, and symlinked destination parents. Existing files require --overwrite.",
    cta: "Read safety",
    screen: "safety"
  }];
  return /*#__PURE__*/React.createElement("section", {
    className: "section section--muted"
  }, /*#__PURE__*/React.createElement("div", {
    className: "section-shell"
  }, /*#__PURE__*/React.createElement("div", {
    className: "section-heading"
  }, /*#__PURE__*/React.createElement("h2", null, "A plan-then-execute file tool for worktrees.")), /*#__PURE__*/React.createElement("div", {
    className: "feature-grid"
  }, features.map(f => /*#__PURE__*/React.createElement("article", {
    key: f.idx,
    className: "feature-card"
  }, /*#__PURE__*/React.createElement("span", {
    className: "feature-card__index"
  }, f.idx), /*#__PURE__*/React.createElement("p", {
    className: "feature-card__eyebrow"
  }, f.eyebrow), /*#__PURE__*/React.createElement("h3", null, f.title), /*#__PURE__*/React.createElement("p", null, f.body), /*#__PURE__*/React.createElement("a", {
    href: "#" + f.screen,
    className: "text-link",
    onClick: e => {
      e.preventDefault();
      onNavigate(f.screen);
    }
  }, f.cta, " \u2192"))))));
}
function DocsPreview({
  onNavigate
}) {
  const docs = [{
    slug: "usage",
    title: "Usage",
    body: "Commands, global options, copy options, typical flow."
  }, {
    slug: "worktreeinclude",
    title: ".worktreeinclude",
    body: "Include file format, eligibility, profile-dependent matching."
  }, {
    slug: "profiles",
    title: "Profiles",
    body: "claude / git / wt — coordinated compatibility presets."
  }, {
    slug: "safety",
    title: "Safety",
    body: "Guarantees: tracked-never-overwritten, symlink-blocked, atomic."
  }, {
    slug: "configuration",
    title: "Configuration",
    body: "Layered config from defaults → user → project → env → CLI."
  }, {
    slug: "architecture",
    title: "Architecture",
    body: "Git is authoritative; matching is pluggable; plan-then-execute."
  }];
  return /*#__PURE__*/React.createElement("section", {
    className: "section"
  }, /*#__PURE__*/React.createElement("div", {
    className: "section-shell"
  }, /*#__PURE__*/React.createElement("div", {
    className: "section-heading"
  }, /*#__PURE__*/React.createElement("h2", null, "Documentation")), /*#__PURE__*/React.createElement("div", {
    className: "docs-grid"
  }, docs.map(d => /*#__PURE__*/React.createElement("article", {
    key: d.slug,
    className: "doc-card"
  }, /*#__PURE__*/React.createElement("h3", null, d.title), /*#__PURE__*/React.createElement("p", null, d.body), /*#__PURE__*/React.createElement("a", {
    href: "#" + d.slug,
    className: "text-link",
    onClick: e => {
      e.preventDefault();
      onNavigate(d.slug);
    }
  }, "Read \u2192"))))));
}
Object.assign(window, {
  FeatureGrid,
  DocsPreview
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/site/Sections.jsx", error: String((e && e.message) || e) }); }

__ds_ns.Badge = __ds_scope.Badge;

__ds_ns.Button = __ds_scope.Button;

__ds_ns.TerminalCard = __ds_scope.TerminalCard;

})();
