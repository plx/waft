import * as React from "react";

export type ButtonVariant = "primary" | "secondary" | "ghost";

export interface ButtonProps extends React.HTMLAttributes<HTMLElement> {
  /** Visual weight. Default "primary". */
  variant?: ButtonVariant;
  /** Renders an <a> when set, otherwise a <button>. */
  href?: string;
  /** Button type when not a link. Default "button". */
  type?: "button" | "submit" | "reset";
  disabled?: boolean;
  children?: React.ReactNode;
}

/** Waft button — primary / secondary / ghost, 36px skeleton. */
export function Button(props: ButtonProps): React.ReactElement;
