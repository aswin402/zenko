"use client";

import type { ComponentProps } from "react";
import { BrainIcon, XIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { field, ghostButton, mono } from "./surfaces";

export type MemoryChange = "added" | "updated" | "existing";

export interface MemoryChip {
  id: string;
  text: string;
  change: MemoryChange;
}

export function MemoryChips({
  chips,
  onForget,
  label,
  compact = false,
  className,
  ...props
}: Omit<ComponentProps<"div">, "children" | "chips" | "onForget" | "label"> & {
  chips: readonly MemoryChip[];
  onForget?: (id: string) => void;
  label?: string;
  compact?: boolean;
}) {
  const fresh = chips.filter((chip) => chip.change !== "existing").length;
  const heading = label ?? (fresh > 0 ? `remembered ${fresh}` : "memory");

  return (
    <div
      data-slot="memory-chips"
      className={cn(
        compact ? "flex flex-wrap items-center gap-1.5" : "flex w-full flex-col gap-2",
        className,
      )}
      {...props}
    >
      <div className="flex items-center gap-1.5">
        <BrainIcon className="text-[var(--accent)] size-3.5 shrink-0" />
        <span className={cn(mono, "text-muted-foreground")}>
          {heading}
        </span>
      </div>

      <div className="flex flex-wrap gap-1.5">
        {chips.map((chip) => (
          <span
            key={chip.id}
            className={cn(
              "fade-in zoom-in-95 animate-in fill-mode-both group flex items-center gap-1 rounded-full py-0.5 text-xs duration-300",
              onForget ? "pr-1 pl-2.5" : "px-2.5",
              chip.change === "existing"
                ? cn(field, "text-muted-foreground")
                : "bg-[color-mix(in_srgb,var(--accent)_13%,transparent)] text-[var(--accent)]",
            )}
          >
            {chip.text}
            {onForget && (
              <button
                type="button"
                aria-label={`Forget "${chip.text}"`}
                onClick={() => onForget(chip.id)}
                className={cn(ghostButton, "size-4 cursor-pointer")}
              >
                <XIcon className="size-2.5" />
              </button>
            )}
          </span>
        ))}
      </div>
    </div>
  );
}
