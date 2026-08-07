const React = (typeof window !== "undefined" && window.React) || globalThis.React;

/**
 * Waft button. Three variants, all sharing the same 36px skeleton.
 * Mirrors .button / .button--primary / .button--secondary from the site.
 */
export function Button({
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
    opacity: disabled ? 0.55 : 1,
  };
  const variants = {
    primary: {
      background: "var(--waft-accent-dark, #0f6e61)",
      borderColor: "var(--waft-accent-dark, #0f6e61)",
      color: "var(--waft-surface, #f7f8f6)",
    },
    secondary: { background: "var(--waft-surface, #f7f8f6)" },
    ghost: { background: "transparent", boxShadow: "none" },
  };
  const style = { ...base, ...(variants[variant] || variants.primary) };
  const Tag = href ? "a" : "button";
  const tagProps = href ? { href } : { type };
  return React.createElement(
    Tag,
    { className: "waft-button", style, onClick, disabled: href ? undefined : disabled, ...tagProps, ...rest },
    children
  );
}
