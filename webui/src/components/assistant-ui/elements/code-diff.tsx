"use client";

import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";
import { codeScroll, codeSurface, mono, paper } from "./surfaces";

export type DiffKind = "context" | "added" | "removed";

export interface DiffLine {
  kind: DiffKind;
  text: string;
}

const GUTTER: Record<DiffKind, string> = {
  context: "",
  added: "+",
  removed: "−",
};

export function CodeDiff({
  filename,
  additions,
  deletions,
  lines,
  cycle,
  className,
  ...props
}: Omit<
  ComponentProps<"div">,
  "children" | "filename" | "additions" | "deletions" | "lines" | "cycle"
> & {
  filename: string;
  additions: number;
  deletions: number;
  lines: readonly DiffLine[];
  cycle: number;
}) {
  return (
    <div
      data-slot="code-diff"
      className={cn(
        paper,
        "w-full overflow-hidden rounded-2xl font-mono text-xs",
        className,
      )}
      {...props}
    >
      <div className="flex items-center justify-between px-4 pt-3 pb-2">
        <span className="truncate text-foreground/90">{filename}</span>
        <span className={cn(mono, "shrink-0 tabular-nums")}>
          <span className="text-[var(--success)]">
            +{additions}
          </span>{" "}
          <span className="text-[var(--destructive)]">−{deletions}</span>
        </span>
      </div>
      <div className={codeScroll}>
        <div className={codeSurface}>
          {lines.map((line, i) => (
            <div
              key={`${cycle}-${i}-${line.text}`}
              className={cn(
                "fade-in animate-in fill-mode-both flex px-4 py-0.5 leading-relaxed whitespace-pre duration-300",
                line.kind === "context" && "text-muted-foreground",
                line.kind === "added" &&
                  "bg-[color-mix(in_srgb,var(--success)_11%,transparent)] text-[var(--fg)]",
                line.kind === "removed" &&
                  "bg-[color-mix(in_srgb,var(--destructive)_11%,transparent)] text-[var(--destructive)]",
              )}
              style={{ animationDelay: `${Math.min(i, 12) * 35}ms` }}
            >
              <span className="w-4 shrink-0 select-none">
                {GUTTER[line.kind]}
              </span>
              <span>{line.text}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
