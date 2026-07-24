const React = (typeof window !== "undefined" && window.React) || globalThis.React;

/**
 * Waft terminal card — the brand's hero element. Mono, header + Copy.
 * Theme-aware: light surface in light mode, dark surface in dark mode.
 * `lines` is an array of command lines; `copyText` overrides what Copy writes.
 */
export function TerminalCard({
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
  return React.createElement(
    "div",
    {
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
        lineHeight: 1.55,
      },
      ...rest,
    },
    React.createElement(
      "div",
      {
        style: {
          minHeight: 40,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 16,
          padding: "0 14px",
          borderBottom: hairline,
          color: dim,
          fontFamily: "var(--waft-font-sans, Inter, system-ui, sans-serif)",
        },
      },
      React.createElement(
        "div",
        null,
        React.createElement(
          "p",
          { style: { margin: "0 0 2px", color: dim, fontSize: "0.7rem", fontWeight: 800, textTransform: "uppercase" } },
          "Terminal"
        ),
        React.createElement("h2", { style: { margin: 0, color: fg, fontSize: "0.95rem", fontWeight: 700 } }, title)
      ),
      React.createElement("span", { style: { color: dim, fontSize: "0.75rem" } }, meta)
    ),
    React.createElement(
      "pre",
      { style: { margin: 0, padding: 14, overflowX: "auto", whiteSpace: "pre-wrap", color: fg } },
      React.createElement("code", null, lines.join("\n"))
    ),
    React.createElement(
      "button",
      {
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
          cursor: "pointer",
        },
      },
      copied ? "Copied" : "Copy"
    )
  );
}
