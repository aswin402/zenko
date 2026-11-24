"use client";

import { type ComponentProps, useMemo } from "react";
import { cn } from "@/lib/utils";
import { take } from "../utils/range";

export interface Segment {
  text: string;
  mono?: boolean;
  bold?: boolean;
}

export function StreamingText({
  segments,
  count,
  streaming,
  className,
  ...props
}: Omit<
  ComponentProps<"p">,
  "children" | "segments" | "count" | "streaming"
> & {
  segments: Segment[];
  count: number;
  streaming: boolean;
}) {
  const words = useMemo(
    () =>
      segments.flatMap((segment) =>
        segment.text
          .split(" ")
          .filter((word) => word.length > 0)
          .map((word) => ({
            word,
            mono: segment.mono ?? false,
            bold: segment.bold ?? false,
          })),
      ),
    [segments],
  );
  const shown = take(words, count);

  return (
    <p
      data-slot="streaming-text"
      className={cn(
        "w-full text-[0.94rem] leading-[1.68] text-pretty text-[var(--fg)]",
        className,
      )}
      {...props}
    >
      {shown.map(({ word, mono: isMono, bold: isBold }, i) => {
        const fresh = streaming && shown.length - 1 - i < 2;
        const inner = isBold ? (
          <strong className="font-semibold">{word}</strong>
        ) : (
          word
        );
        return (
          <span
            key={i}
            className="fade-in animate-in fill-mode-both duration-500 motion-reduce:animate-none"
          >
            <span
              className={cn(
                "transition-colors duration-700 motion-reduce:transition-none",
                fresh && "text-[var(--accent)]",
                isMono &&
                  "rounded-md bg-[color-mix(in_srgb,var(--accent)_13%,var(--card))] px-1.5 py-0.5 font-mono text-[0.85em] text-[color-mix(in_srgb,var(--accent)_78%,var(--fg))]",
              )}
            >
              {inner}
            </span>{" "}
          </span>
        );
      })}
      {streaming && shown.length > 0 && (
        <span
          aria-hidden
          className="-mb-0.5 ml-0.5 inline-block h-4 w-0.5 animate-pulse rounded-full bg-[var(--accent)]"
        />
      )}
    </p>
  );
}
