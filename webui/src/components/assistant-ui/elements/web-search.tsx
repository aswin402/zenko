"use client";

import type { ComponentProps } from "react";
import { SearchIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { field, mono, ShimmerLabel } from "./surfaces";
import { safeHref } from "../utils/href";
import { take } from "../utils/range";

export interface WebSearchResult {
  title: string;
  domain: string;
  url?: string;
}

export function WebSearch({
  query,
  results,
  visibleResults,
  searching,
  cycle,
  className,
  ...props
}: Omit<
  ComponentProps<"div">,
  "children" | "query" | "results" | "visibleResults" | "searching" | "cycle"
> & {
  query: string;
  results: readonly WebSearchResult[];
  visibleResults: number;
  searching: boolean;
  cycle: number;
}) {
  return (
    <div
      data-slot="web-search"
      className={cn("flex w-full flex-col gap-2.5", className)}
      {...props}
    >
      <span
        className={cn(
          field,
          "text-foreground/75 inline-flex w-fit items-center gap-1.5 rounded-full px-3.5 py-1.5 text-xs",
        )}
      >
        <SearchIcon className="text-[var(--accent)] size-3 shrink-0" />
        <span className="truncate">{query}</span>
      </span>
      <div className="text-muted-foreground text-xs">
        {searching ? (
          <ShimmerLabel className="relative inline-block leading-none">
            Searching
          </ShimmerLabel>
        ) : (
          <span className="fade-in animate-in duration-300">
            {results.length === 0
              ? "No sources found"
              : `Read ${results.length} ${results.length === 1 ? "source" : "sources"}`}
          </span>
        )}
      </div>
      {results.length > 0 && (
        <div className="flex flex-col">
          {take(results, visibleResults).map((result, idx) => {
            const href = safeHref(result.url);
            const rowBody = (
              <>
                <span className="bg-foreground/[0.06] text-muted-foreground flex size-4 shrink-0 items-center justify-center rounded text-[9px] font-medium">
                  {result.domain.charAt(0).toUpperCase()}
                </span>
                <span className="text-foreground/90 min-w-0 flex-1 truncate text-[13px]">
                  {result.title}
                </span>
                <span className={cn(mono, "text-muted-foreground shrink-0")}>
                  {result.domain}
                </span>
              </>
            );
            const rowClass =
              "fade-in slide-in-from-bottom-1 animate-in fill-mode-both hover:bg-foreground/[0.04] -mx-2 flex items-center gap-2.5 rounded-xl px-2 py-1.5 transition-colors duration-200";
            return href ? (
              <a
                key={`${cycle}-${idx}-${result.domain}`}
                href={href}
                target="_blank"
                rel="noopener noreferrer"
                className={rowClass}
              >
                {rowBody}
              </a>
            ) : (
              <div key={`${cycle}-${idx}-${result.domain}`} className={rowClass}>
                {rowBody}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
