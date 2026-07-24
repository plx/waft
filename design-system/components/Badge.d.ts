import * as React from "react";

export type BadgeTone = "neutral" | "accent" | "warn";

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  /** "neutral" tags, "accent" eligible, "warn" excluded. Default "neutral". */
  tone?: BadgeTone;
  children?: React.ReactNode;
}

/** Waft badge / pill — mono, 12px, hairline border. */
export function Badge(props: BadgeProps): React.ReactElement;
