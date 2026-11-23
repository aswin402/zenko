"use client";

import type { ComponentProps } from "react";
import {
  CheckIcon,
  CopyIcon,
  EllipsisIcon,
  RotateCcw,
} from "lucide-react";
import { ZenkoTooltip } from "@/components/ui/Tooltip";
import { cn } from "@/lib/utils";
import { ghostButton, iconSwap, iconSwapIn, iconSwapOut } from "./surfaces";

export interface MessageActionsProps extends Omit<
  ComponentProps<"div">,
  "children"
> {
  copied: boolean;
  regenerating: boolean;
  onCopy: () => void;
  onRegenerate?: () => void;
  onMore?: () => void;
}

export function MessageActions({
  copied,
  regenerating,
  onCopy,
  onRegenerate,
  onMore,
  className,
  ...props
}: MessageActionsProps) {
  const buttonClassName = cn(ghostButton, "size-7");

  return (
    <div
      data-slot="message-actions"
      className={cn("flex items-center gap-1", className)}
      {...props}
    >
      <ZenkoTooltip label={copied ? "Copied" : "Copy response"} side="top">
        <button
          type="button"
          aria-label={copied ? "Copied response" : "Copy response"}
          onClick={onCopy}
          className={cn(
            buttonClassName,
            "grid place-items-center",
            copied && "text-[var(--accent)]",
          )}
        >
          <CopyIcon
            className={cn(
              iconSwap,
              "size-3.5",
              copied ? iconSwapOut : iconSwapIn,
            )}
          />
          <CheckIcon
            className={cn(
              iconSwap,
              "size-3.5",
              copied ? iconSwapIn : iconSwapOut,
            )}
          />
        </button>
      </ZenkoTooltip>
      {onRegenerate ? (
        <ZenkoTooltip label="Retry for a better answer" side="top">
          <button
            type="button"
            aria-label="Retry for a better answer"
            onClick={onRegenerate}
            className={buttonClassName}
          >
            <RotateCcw
              className={cn(
                "size-3.5",
                regenerating && "animate-spin motion-reduce:animate-none",
              )}
            />
          </button>
        </ZenkoTooltip>
      ) : null}
      {onMore ? (
        <ZenkoTooltip label="Toggle reasoning & steps" side="top">
          <button
            type="button"
            aria-label="Toggle reasoning & steps"
            onClick={onMore}
            className={buttonClassName}
          >
            <EllipsisIcon className="size-3.5" />
          </button>
        </ZenkoTooltip>
      ) : null}
    </div>
  );
}
