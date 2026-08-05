import * as React from "react";

export interface TerminalCardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Card title, shown bold in the header. Default "waft". */
  title?: string;
  /** Right-aligned header meta. Default "copy commands". */
  meta?: string;
  /** Command lines to render in the body. */
  lines?: string[];
  /** Text the Copy button writes; defaults to lines joined by newlines. */
  copyText?: string;
}

/** Waft terminal card — theme-aware mono command block with header + Copy button. */
export function TerminalCard(props: TerminalCardProps): React.ReactElement;
