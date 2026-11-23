"use client";

import type { ReactNode } from "react";
import { CheckIcon, ChevronRightIcon, LoaderCircle, XIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  CollapsibleRoot as Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./collapsible-root";
import {
  collapsePanel,
  field,
  mono,
  ShimmerLabel,
  SwapLabel,
} from "./surfaces";

export interface ToolCallProps {
  label: string;
  activeLabel: string;
  query: string;
  request: string;
  result: string;
  running: boolean;
  failed?: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  leadingIcon?: ReactNode;
  className?: string;
}

export function ToolCall({
  label,
  activeLabel,
  query,
  request,
  result,
  running,
  failed = false,
  open,
  onOpenChange,
  leadingIcon,
  className,
}: ToolCallProps) {
  return (
    <Collapsible
      data-slot="tool-call"
      open={open}
      onOpenChange={onOpenChange}
      className={cn("w-full", className)}
    >
      <CollapsibleTrigger className="group/trigger text-muted-foreground hover:text-foreground/90 flex w-full cursor-pointer items-center gap-2 rounded-md py-1 text-[13px] transition-colors outline-none">
        {leadingIcon ? (
          <span
            aria-hidden="true"
            className="grid size-4 shrink-0 place-items-center text-muted-foreground transition-colors group-hover/trigger:text-foreground/85 group-data-[state=open]/trigger:text-[var(--accent)]"
          >
            {leadingIcon}
          </span>
        ) : (
          <ChevronRightIcon className="size-3.5 shrink-0 opacity-60 transition-transform duration-200 ease-[cubic-bezier(0.32,0.72,0,1)] group-data-[state=open]/trigger:rotate-90 motion-reduce:transition-none" />
        )}
        <SwapLabel
          active={running ? 0 : 1}
          className="shrink-0 text-start whitespace-nowrap"
        >
          <ShimmerLabel
            active={running}
            className="relative inline-block leading-none"
          >
            {activeLabel}
          </ShimmerLabel>
          <>{label}</>
        </SwapLabel>
        {query ? (
          <span
            className={cn(
              mono,
              "bg-foreground/[0.06] text-foreground/70 min-w-0 truncate rounded-md px-1.5 py-0.5",
            )}
          >
            {query}
          </span>
        ) : null}
        <span className="ms-auto flex w-4 shrink-0 items-center justify-end">
          {running ? (
            <LoaderCircle className="size-3.5 animate-spin text-[var(--accent)]" />
          ) : failed ? (
            <XIcon
              role="img"
              aria-label="Failed"
              className="fade-in zoom-in-90 animate-in size-3.5 text-[var(--destructive)] duration-200"
            />
          ) : (
            <CheckIcon className="fade-in zoom-in-90 animate-in size-3.5 text-[var(--accent)] duration-200" />
          )}
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent className={cn(collapsePanel, "outline-none")}>
        <div className={cn(field, "mt-2 overflow-hidden rounded-2xl text-xs")}>
          {request ? (
            <>
              <div className="px-3.5 pt-2.5 pb-2">
                <p className={cn(mono, "text-muted-foreground mb-1")}>Request</p>
                <p className="text-muted-foreground font-mono break-words whitespace-pre-wrap">
                  {request}
                </p>
              </div>
              <div className="bg-foreground/[0.06] mx-3.5 h-px" />
            </>
          ) : null}
          <div className="px-3.5 pt-2 pb-2.5">
            <p className={cn(mono, "text-muted-foreground mb-1")}>Result</p>
            <p className="text-foreground/90 font-mono break-words whitespace-pre-wrap">{result}</p>
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
