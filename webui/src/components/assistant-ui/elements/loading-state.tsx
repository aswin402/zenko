"use client";

import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";
import { ShimmerLabel } from "./surfaces";

export type GenerationLoaderVariant = "dots" | "squares" | "rounded";

export interface GenerationLoaderProps extends Omit<
  ComponentProps<"div">,
  "children"
> {
  label: string;
  tick: number;
  variant?: GenerationLoaderVariant;
  inline?: boolean;
}

const CELL_SHAPES: Record<GenerationLoaderVariant, string> = {
  dots: "rounded-full",
  squares: "rounded-[1px]",
  rounded: "rounded-[3px]",
};

export function GenerationLoader({
  label,
  tick,
  variant = "dots",
  inline = false,
  className,
  ...props
}: GenerationLoaderProps) {
  const pixelOffset = Math.floor(tick / 3);

  return (
    <div
      data-slot="generation-loader"
      className={cn(
        inline ? "inline-flex items-center gap-2.5" : "flex flex-col items-center gap-4",
        className,
      )}
      {...props}
    >
      <div aria-hidden className={cn("grid grid-cols-3", inline ? "gap-0.5" : "gap-1")}>
        {Array.from({ length: 9 }, (_, index) => {
          const active = (index * 2 + pixelOffset) % 9 < 3;

          return (
            <span
              key={index}
              className={cn(
                "bg-[var(--accent)] transition-opacity duration-300 motion-reduce:transition-none",
                inline ? "size-1" : "size-2",
                CELL_SHAPES[variant],
                active ? "opacity-95" : "opacity-20",
              )}
            />
          );
        })}
      </div>
      {label ? (
        <ShimmerLabel
          className={cn(
            "text-muted-foreground relative inline-block",
            inline ? "text-[13px]" : "text-sm",
          )}
        >
          {label}
        </ShimmerLabel>
      ) : null}
    </div>
  );
}
