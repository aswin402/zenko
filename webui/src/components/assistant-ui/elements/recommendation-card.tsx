"use client";

import type { ComponentProps } from "react";
import type { ReactNode } from "react";
import { CheckIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { inkButton, mono } from "./surfaces";
import { useReceiptFocus } from "./receipt-focus";

export type RecommendationState = "idle" | "accepted";

const CONFIDENCE_BARS = [0, 1, 2];

export function RecommendationCard({
  state,
  question,
  children,
  confidenceLabel,
  confidenceLevel = 3,
  acceptedLabel,
  acceptLabel = "Accept",
  alternativesLabel = "Alternatives",
  onAccept,
  onAlternatives,
  className,
  ...props
}: Omit<
  ComponentProps<"div">,
  | "state"
  | "question"
  | "children"
  | "confidenceLabel"
  | "acceptedLabel"
  | "onAccept"
  | "onAlternatives"
> & {
  state: RecommendationState;
  question: string;
  children: ReactNode;
  confidenceLabel: string;
  confidenceLevel?: 1 | 2 | 3;
  acceptedLabel: string;
  acceptLabel?: string;
  alternativesLabel?: string;
  onAccept?: () => void;
  onAlternatives?: () => void;
}) {
  const { receiptRef, focusHandlers } = useReceiptFocus<HTMLDivElement>(props);
  return (
    <div
      data-slot="recommendation-card"
      className={cn(
        "flex w-full max-w-sm flex-col gap-3",
        className,
      )}
      {...props}
      {...focusHandlers}
    >
      <p className="text-sm font-medium text-[var(--fg)]">{question}</p>
      <div className="text-muted-foreground text-[13px] leading-relaxed">
        {children}
      </div>

      <div className="flex h-8 items-center justify-between gap-2">
        {state === "idle" ? (
          <>
            <div className="flex items-center gap-2">
              <span className="flex items-end gap-0.5" aria-hidden>
                {CONFIDENCE_BARS.map((bar) => (
                  <span
                    key={bar}
                    className={cn(
                      "w-1 rounded-full",
                      bar < confidenceLevel
                        ? "bg-[var(--accent)]/85"
                        : "bg-[var(--border)]",
                    )}
                    style={{ height: 6 + bar * 3 }}
                  />
                ))}
              </span>
              <span className={cn(mono, "text-muted-foreground")}>
                {confidenceLabel}
              </span>
            </div>
            <div className="flex items-center gap-2">
              {onAlternatives ? (
                <button
                  type="button"
                  onClick={onAlternatives}
                  className="text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground/90 h-8 cursor-pointer rounded-full px-3.5 text-xs font-medium transition-[background-color,color,scale] duration-150 active:scale-[0.96]"
                >
                  {alternativesLabel}
                </button>
              ) : null}
              {onAccept ? (
                <button
                  type="button"
                  onClick={onAccept}
                  className={cn(
                    inkButton,
                    "flex h-8 cursor-pointer items-center rounded-full px-3.5 text-xs font-medium",
                  )}
                >
                  {acceptLabel}
                </button>
              ) : null}
            </div>
          </>
        ) : (
          <div
            key="accepted"
            ref={receiptRef}
            tabIndex={-1}
            className="fade-in animate-in text-muted-foreground flex items-center gap-2 text-xs duration-300"
          >
            <CheckIcon className="size-3.5 text-[var(--accent)]" />
            {acceptedLabel}
          </div>
        )}
      </div>
    </div>
  );
}
