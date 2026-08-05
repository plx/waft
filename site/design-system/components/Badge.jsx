const React = (typeof window !== "undefined" && window.React) || globalThis.React;

/**
 * Waft badge / pill. Mono, 12px, hairline border.
 * tone: "neutral" (tags), "accent" (eligible), "warn" (excluded).
 */
export function Badge({ tone = "neutral", children, ...rest }) {
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
    whiteSpace: "nowrap",
  };
  const tones = {
    neutral: {},
    accent: {
      color: "var(--waft-accent-dark, #0f6e61)",
      borderColor: "color-mix(in srgb, var(--waft-accent, #16a085) 42%, transparent)",
      background: "var(--waft-accent-soft, #dff5f1)",
    },
    warn: {
      color: "var(--waft-error, #b42323)",
      borderColor: "color-mix(in srgb, var(--waft-error, #b42323) 34%, #e2e5e9)",
      background: "var(--waft-error-bg, #fce4e4)",
    },
  };
  const style = { ...base, ...(tones[tone] || tones.neutral) };
  return React.createElement("span", { className: "waft-badge", style, ...rest }, children);
}
