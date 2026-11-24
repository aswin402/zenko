"use client";
// beui.dev/components/motion/theme-toggle (adapted for Zenko native theme engine — zero next-themes)

import { Moon, Sun } from "lucide-react";
import { useReducedMotion } from "motion/react";
import { type ComponentPropsWithoutRef, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import { ActionSwapIcon } from "@/components/motion/action-swap";
import {
  Skiper4ToggleIcon,
  type ThemeToggleIconVariant,
} from "@/components/skiperui/theme-toggle-icons";
import { EASE_OUT_CSS } from "@/lib/ease";
import { useMessages } from "@/lib/i18n";
import { cn } from "@/lib/utils";

export type ThemeVariant =
  | "rectangle"
  | "circle"
  | "circle-blur"
  | "blinds"
  | "skiper26_bottom_up"
  | "skiper26_top_down"
  | "skiper26_diagonal";

export type RectStart =
  | "top-left"
  | "top-right"
  | "bottom-left"
  | "bottom-right"
  | "center"
  | "bottom-up";

/** English defaults. Override per instance with `labels`, or app-wide with `LocaleProvider`. */
export const themeToggleMessages = {
  switchToLight: "Switch to light mode",
  switchToDark: "Switch to dark mode",
};

export type ThemeToggleMessages = typeof themeToggleMessages;

export interface ThemeToggleProps
  extends Omit<ComponentPropsWithoutRef<"button">, "children" | "onClick"> {
  /** Current theme mode from Zenko UI state. */
  mode?: "dark" | "light";
  /** Callback that switches Zenko theme mode (called inside View Transition). */
  onToggle?: () => void;
  /** Animation variant. Default: "circle-blur". */
  variant?: ThemeVariant;
  /** Origin direction for the reveal. Default: "top-right". */
  start?: RectStart;
  /** Animated icon variant (`@skiper-ui/skiper4`). Defaults to global `motion.theme_toggle_icon`. */
  iconVariant?: ThemeToggleIconVariant;
  iconClassName?: string;
  /** Override any built-in string. */
  labels?: Partial<ThemeToggleMessages>;
}

const VT_STYLE_ID = "beui-theme-toggle-vt";

const VT_CSS = `
html[data-beui-vt="rect"]::view-transition-old(root) {
  animation: none;
  mix-blend-mode: normal;
}
html[data-beui-vt="rect"]::view-transition-new(root) {
  mix-blend-mode: normal;
  animation: beui-rect-reveal 400ms ease-out;
}
html[data-beui-vt="circle"]::view-transition-old(root),
html[data-beui-vt="circle-blur"]::view-transition-old(root) {
  animation: none;
  mix-blend-mode: normal;
}
html[data-beui-vt="circle"]::view-transition-new(root) {
  mix-blend-mode: normal;
  animation: beui-circle-reveal 680ms cubic-bezier(0.4, 0, 0.2, 1);
}
html[data-beui-vt="circle-blur"]::view-transition-new(root) {
  mix-blend-mode: normal;
  animation: beui-circle-blur-reveal 680ms cubic-bezier(0.4, 0, 0.2, 1);
}
html[data-beui-vt="blinds"]::view-transition-old(root) {
  animation: none;
  mix-blend-mode: normal;
}
@property --beui-vt-slat {
  syntax: "<length>";
  inherits: false;
  initial-value: 72px;
}
html[data-beui-vt="blinds"]::view-transition-new(root) {
  mix-blend-mode: normal;
  mask-image: linear-gradient(
    90deg,
    #000 0 var(--beui-vt-slat),
    transparent calc(var(--beui-vt-slat) + 20px)
  );
  mask-size: 72px 100%;
  mask-repeat: repeat;
  animation: beui-blinds-reveal 680ms ${EASE_OUT_CSS};
}
@keyframes beui-rect-reveal {
  from { clip-path: var(--beui-vt-from, inset(100% 0 0 0)); }
  to   { clip-path: inset(0 0 0 0); }
}
@keyframes beui-circle-reveal {
  from { clip-path: circle(0% at var(--beui-vt-origin, 50% 100%)); }
  to   { clip-path: circle(150% at var(--beui-vt-origin, 50% 100%)); }
}
@keyframes beui-circle-blur-reveal {
  from { clip-path: circle(0% at var(--beui-vt-origin, 50% 100%)); filter: blur(10px); }
  to   { clip-path: circle(150% at var(--beui-vt-origin, 50% 100%)); filter: blur(0px); }
}
@keyframes beui-blinds-reveal {
  from { --beui-vt-slat: -20px; }
  to   { --beui-vt-slat: 72px; }
}
html[data-beui-vt="skiper26-bottom-up"]::view-transition-old(root),
html[data-beui-vt="skiper26-top-down"]::view-transition-old(root),
html[data-beui-vt="skiper26-diagonal"]::view-transition-old(root) {
  animation: none;
  mix-blend-mode: normal;
}
html[data-beui-vt="skiper26-bottom-up"]::view-transition-new(root) {
  mix-blend-mode: normal;
  animation: skiper26-bottom-up-reveal 560ms cubic-bezier(0.22, 1, 0.36, 1);
}
html[data-beui-vt="skiper26-top-down"]::view-transition-new(root) {
  mix-blend-mode: normal;
  animation: skiper26-top-down-reveal 560ms cubic-bezier(0.22, 1, 0.36, 1);
}
html[data-beui-vt="skiper26-diagonal"]::view-transition-new(root) {
  mix-blend-mode: normal;
  animation: skiper26-diagonal-reveal 620ms cubic-bezier(0.22, 1, 0.36, 1);
}
@keyframes skiper26-bottom-up-reveal {
  from { clip-path: inset(100% 0 0 0); }
  to   { clip-path: inset(0 0 0 0); }
}
@keyframes skiper26-top-down-reveal {
  from { clip-path: inset(0 0 100% 0); }
  to   { clip-path: inset(0 0 0 0); }
}
@keyframes skiper26-diagonal-reveal {
  from { clip-path: polygon(0 0, 0 0, 0 0, 0 0); }
  to   { clip-path: polygon(0 0, 200% 0, 100% 200%, 0 100%); }
}
`;

const RECT_FROM: Record<RectStart, string> = {
  "top-left": "inset(0 100% 100% 0)",
  "top-right": "inset(0 0 100% 100%)",
  "bottom-left": "inset(100% 100% 0 0)",
  "bottom-right": "inset(100% 0 0 100%)",
  center: "inset(50% 50% 50% 50%)",
  "bottom-up": "inset(100% 0 0 0)",
};

const CIRCLE_ORIGIN: Record<RectStart, string> = {
  "top-left": "0% 0%",
  "top-right": "100% 0%",
  "bottom-left": "0% 100%",
  "bottom-right": "100% 100%",
  center: "50% 50%",
  "bottom-up": "50% 100%",
};

export function ensureThemeToggleStyles() {
  if (typeof document === "undefined") return;
  if (document.getElementById(VT_STYLE_ID)) return;
  const el = document.createElement("style");
  el.id = VT_STYLE_ID;
  el.textContent = VT_CSS;
  document.head.appendChild(el);
}

/**
 * Triggers a View Transitions API reveal (defaulting to BeUI circle-blur) around
 * any synchronous theme mutation callback. Optionally anchors the circle at click coordinates.
 */
export function runThemeViewTransition(
  apply: () => void,
  options?: {
    variant?: ThemeVariant;
    start?: RectStart;
    origin?: { x: number; y: number };
    reduce?: boolean;
  },
) {
  const variant = options?.variant ?? "circle-blur";
  const start = options?.start ?? "top-right";
  const reduce = options?.reduce ?? false;

  ensureThemeToggleStyles();

  if (
    reduce ||
    typeof document === "undefined" ||
    !("startViewTransition" in document)
  ) {
    apply();
    return;
  }

  const root = document.documentElement;
  if (variant === "rectangle") {
    root.style.setProperty("--beui-vt-from", RECT_FROM[start]);
    root.dataset.beuiVt = "rect";
  } else if (variant === "blinds") {
    root.dataset.beuiVt = "blinds";
  } else if (variant === "skiper26_bottom_up") {
    root.dataset.beuiVt = "skiper26-bottom-up";
  } else if (variant === "skiper26_top_down") {
    root.dataset.beuiVt = "skiper26-top-down";
  } else if (variant === "skiper26_diagonal") {
    root.dataset.beuiVt = "skiper26-diagonal";
  } else {
    const originStr = options?.origin
      ? `${options.origin.x}px ${options.origin.y}px`
      : CIRCLE_ORIGIN[start];
    root.style.setProperty("--beui-vt-origin", originStr);
    root.dataset.beuiVt = variant;
  }

  const vt = (
    document as Document & {
      startViewTransition(cb: () => void): { finished: Promise<void> };
    }
  ).startViewTransition(() => {
    flushSync(() => {
      apply();
    });
  });

  vt.finished.finally(() => {
    delete root.dataset.beuiVt;
  });
}

export function useThemeToggle({
  mode = "dark",
  onToggle,
  variant = "circle-blur",
  start = "top-right",
}: {
  mode?: "dark" | "light";
  onToggle?: () => void;
  variant?: ThemeVariant;
  start?: RectStart;
} = {}) {
  const reduce = useReducedMotion() ?? false;
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
    ensureThemeToggleStyles();
  }, []);
  const isDark = mounted && mode !== "light";

  const toggle = (event?: React.MouseEvent<HTMLButtonElement>) => {
    const origin =
      event && (event.clientX > 0 || event.clientY > 0)
        ? { x: event.clientX, y: event.clientY }
        : undefined;
    runThemeViewTransition(
      () => {
        onToggle?.();
      },
      { variant, start, origin, reduce },
    );
  };

  return { isDark, mounted, toggle };
}

function useGlobalToggleIconVariant(override?: ThemeToggleIconVariant): ThemeToggleIconVariant {
  const [iconStyle, setIconStyle] = useState<ThemeToggleIconVariant>(() => {
    if (override) return override;
    if (typeof document === "undefined") return "default";
    return (
      (document.documentElement.dataset.themeToggleIcon as ThemeToggleIconVariant | undefined) ||
      "default"
    );
  });

  useEffect(() => {
    if (override) {
      setIconStyle(override);
      return;
    }
    if (typeof document === "undefined") return;
    const sync = () => {
      setIconStyle(
        (document.documentElement.dataset.themeToggleIcon as ThemeToggleIconVariant | undefined) ||
          "default",
      );
    };
    sync();
    const obs = new MutationObserver(sync);
    obs.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme-toggle-icon"],
    });
    return () => obs.disconnect();
  }, [override]);

  return override ?? iconStyle;
}

export function ThemeToggle({
  mode = "dark",
  onToggle,
  variant = "circle-blur",
  start = "top-right",
  iconVariant: iconOverride,
  className,
  iconClassName,
  labels,
  ...rest
}: ThemeToggleProps) {
  const t = useMessages("themeToggle", themeToggleMessages, labels);
  const { isDark, mounted, toggle } = useThemeToggle({
    mode,
    onToggle,
    variant,
    start,
  });
  const activeIconVariant = useGlobalToggleIconVariant(iconOverride);

  return (
    <button
      type="button"
      aria-label={t(mounted && isDark ? "switchToLight" : "switchToDark")}
      onClick={toggle}
      className={cn("flex cursor-pointer items-center justify-center", className)}
      {...rest}
    >
      {mounted ? (
        activeIconVariant !== "default" ? (
          <Skiper4ToggleIcon
            variant={activeIconVariant}
            isDark={isDark}
            className={iconClassName}
          />
        ) : (
          <ActionSwapIcon
            value={isDark ? "dark" : "light"}
            animation="blur"
            className={iconClassName}
          >
            {isDark ? (
              <Sun className={cn("size-4", iconClassName)} />
            ) : (
              <Moon className={cn("size-4", iconClassName)} />
            )}
          </ActionSwapIcon>
        )
      ) : (
        <span className={cn("size-4", iconClassName)} aria-hidden="true" />
      )}
    </button>
  );
}
