"use client";

import type { ComponentProps } from "react";
import { CircleAlertIcon, RefreshCwIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { ShimmerLabel } from "./surfaces";

export interface ErrorStateProps extends Omit<
  ComponentProps<"div">,
  "children" | "role"
> {
  title: string;
  detail: string;
  retrying: boolean;
  onRetry: () => void;
}

export function ErrorState({
  title,
  detail,
  retrying,
  onRetry,
  className,
  ...props
}: ErrorStateProps) {
  if (retrying) {
    return (
      <div
        data-slot="error-state"
        key="retrying"
        role="status"
        className={cn(
          "fade-in animate-in flex w-full items-center gap-2.5 text-sm duration-300 motion-reduce:animate-none",
          className,
        )}
        {...props}
      >
        <RefreshCwIcon className="text-muted-foreground size-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
        <ShimmerLabel className="text-muted-foreground relative inline-block">
          Retrying
        </ShimmerLabel>
      </div>
    );
  }

  return (
    <div
      data-slot="error-state"
      key="error"
      role="alert"
      className={cn(
        "zenko-aui-paper fade-in animate-in flex w-full items-start gap-2.5 rounded-2xl border border-[color-mix(in_srgb,var(--destructive)_26%,transparent)] bg-[color-mix(in_srgb,var(--destructive)_8%,transparent)] px-4 py-3 text-sm duration-300 motion-reduce:animate-none",
        className,
      )}
      {...props}
    >
      <CircleAlertIcon className="mt-0.5 size-4 shrink-0 text-[var(--destructive)]" />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-[var(--destructive)]">{title}</p>
        <p className="mt-0.5 break-words text-[13px] leading-snug text-[var(--destructive)]/75">
          {detail}
        </p>
      </div>
      <button
        type="button"
        onClick={onRetry}
        className="ms-auto flex shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium text-[var(--destructive)] transition-colors hover:bg-[color-mix(in_srgb,var(--destructive)_12%,transparent)]"
      >
        <RefreshCwIcon className="size-3" />
        Retry
      </button>
    </div>
  );
}
