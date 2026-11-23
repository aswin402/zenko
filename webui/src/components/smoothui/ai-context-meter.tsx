"use client";

import { cn } from "@/lib/utils";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useState } from "react";

const SPRING_DEFAULT = {
  bounce: 0.1,
  duration: 0.25,
  type: "spring" as const,
};
const EASE_OUT = [0.23, 1, 0.32, 1] as const;
const VIEWBOX = 32;
const CENTER = VIEWBOX / 2;
const RADIUS = 13;
const STROKE_WIDTH = 3;
/** Fraction of the window at which the ring changes hue. */
const DEFAULT_WARNING_AT = 0.8;
const DEFAULT_DANGER_AT = 0.95;
const WARNING_COLOR = "var(--accent)";
const DANGER_COLOR = "var(--destructive)";

export type AIContextBreakdownItem = {
  label: string;
  tokens: number;
};

export type ContextMeterChrome =
  | "bordered_filled"
  | "border_only"
  | "background_only"
  | "minimal_bare";

export const CONTEXT_METER_CHROME_CLASS: Record<ContextMeterChrome, string> = {
  bordered_filled:
    "rounded-full border border-[var(--border)] bg-[var(--bg)]/80 px-2.5 py-1 hover:border-[var(--accent)]/45",
  border_only:
    "rounded-full border border-[var(--border)] bg-transparent px-2.5 py-1 hover:border-[var(--accent)]/45",
  background_only:
    "rounded-full border border-transparent bg-[var(--bg)]/80 px-2.5 py-1 hover:bg-[var(--card)]",
  minimal_bare:
    "rounded-lg border border-transparent bg-transparent px-1 py-0.5",
};

export type AIContextMeterProps = {
  /** Optional split of what is filling the window. */
  breakdown?: AIContextBreakdownItem[];
  chrome?: ContextMeterChrome;
  className?: string;
  /** Fraction at which the ring turns red. */
  dangerAt?: number;
  /** Total size of the context window, in tokens. */
  limit: number;
  /** Optional external click action (e.g. open full context inspector). */
  onTriggerClick?: () => void;
  /** Tokens currently used. */
  used: number;
  /** Fraction at which the ring turns accent warning. */
  warningAt?: number;
};

const COMPACT_THRESHOLD = 1000;
const MILLION = 1_000_000;

/** Below this many thousands, keep a decimal — "2k" for 1,800 is a lie. */
const DECIMAL_BELOW = 10 * COMPACT_THRESHOLD;

const formatTokens = (tokens: number): string => {
  if (tokens >= MILLION) {
    return `${(tokens / MILLION).toFixed(1)}M`;
  }
  if (tokens >= DECIMAL_BELOW) {
    return `${Math.round(tokens / COMPACT_THRESHOLD)}k`;
  }
  if (tokens >= COMPACT_THRESHOLD) {
    return `${(tokens / COMPACT_THRESHOLD).toFixed(1)}k`;
  }
  return String(tokens);
};

/**
 * How much of the context window is gone.
 *
 * Crossing a threshold changes the **hue**, never the size. Growing the ring at
 * the warning point would read as progress — as something filling up nicely —
 * which is the opposite of the message. The geometry stays put and the colour
 * does the talking.
 */
const AIContextMeter = ({
  breakdown,
  chrome = "minimal_bare",
  className,
  dangerAt = DEFAULT_DANGER_AT,
  limit,
  onTriggerClick,
  used,
  warningAt = DEFAULT_WARNING_AT,
}: AIContextMeterProps) => {
  const shouldReduceMotion = useReducedMotion();
  const [isOpen, setIsOpen] = useState(false);

  const fraction = limit > 0 ? Math.min(1, Math.max(0, used / limit)) : 0;
  const percent = Math.round(fraction * 100);

  const color = (() => {
    if (fraction >= dangerAt) {
      return DANGER_COLOR;
    }
    if (fraction >= warningAt) {
      return WARNING_COLOR;
    }
    return "var(--accent)";
  })();

  const hasBreakdown = Boolean(breakdown?.length);

  return (
    <div className={cn("relative inline-block", className)}>
      <button
        aria-expanded={hasBreakdown ? isOpen : undefined}
        aria-label={`Context window ${percent}% used, ${formatTokens(used)} of ${formatTokens(limit)} tokens`}
        className={cn(
          "flex cursor-pointer items-center gap-1.5 font-mono text-[11px] text-[var(--muted)] transition-colors hover:text-[var(--fg)]",
          CONTEXT_METER_CHROME_CLASS[chrome],
        )}
        onBlur={() => setIsOpen(false)}
        onClick={() => {
          if (onTriggerClick) onTriggerClick();
          else if (hasBreakdown) setIsOpen((current) => !current);
        }}
        onFocus={() => hasBreakdown && setIsOpen(true)}
        onMouseEnter={() => hasBreakdown && setIsOpen(true)}
        onMouseLeave={() => setIsOpen(false)}
        type="button"
      >
        <svg
          aria-hidden="true"
          className="size-4 -rotate-90"
          viewBox={`0 0 ${VIEWBOX} ${VIEWBOX}`}
        >
          <circle
            cx={CENTER}
            cy={CENTER}
            fill="none"
            r={RADIUS}
            stroke="currentColor"
            strokeOpacity={0.2}
            strokeWidth={STROKE_WIDTH}
          />
          {/* pathLength normalises the dash maths, so the fill is just the
              fraction — no circumference arithmetic to get wrong. */}
          <motion.circle
            animate={{
              stroke: color,
              strokeDasharray: `${fraction} ${1 - fraction}`,
            }}
            cx={CENTER}
            cy={CENTER}
            fill="none"
            pathLength={1}
            r={RADIUS}
            strokeLinecap="round"
            strokeWidth={STROKE_WIDTH}
            transition={shouldReduceMotion ? { duration: 0 } : SPRING_DEFAULT}
          />
        </svg>

        <span className="tabular-nums">
          {formatTokens(used)}/{formatTokens(limit)}
        </span>
      </button>

      <AnimatePresence>
        {isOpen && hasBreakdown ? (
          <motion.div
            animate={{ opacity: 1, scale: 1, y: 0 }}
            className="absolute bottom-full right-0 z-50 mb-1.5 w-56 rounded-xl border border-[var(--border)] bg-[var(--surface)] p-2.5 shadow-xl"
            exit={
              shouldReduceMotion
                ? { opacity: 0, transition: { duration: 0 } }
                : { opacity: 0, scale: 0.96, y: 4 }
            }
            initial={
              shouldReduceMotion
                ? { opacity: 1, scale: 1, y: 0 }
                : { opacity: 0, scale: 0.96, y: 4 }
            }
            style={{ transformOrigin: "bottom left" }}
            transition={
              shouldReduceMotion
                ? { duration: 0 }
                : { duration: 0.18, ease: EASE_OUT }
            }
          >
            <ul className="list-none space-y-1">
              {breakdown?.map((item) => (
                <li
                  className="flex items-baseline justify-between gap-3 text-xs"
                  key={item.label}
                >
                  <span className="truncate text-muted-foreground">
                    {item.label}
                  </span>
                  <span className="shrink-0 text-foreground tabular-nums">
                    {formatTokens(item.tokens)}
                  </span>
                </li>
              ))}
            </ul>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
};

export default AIContextMeter;
