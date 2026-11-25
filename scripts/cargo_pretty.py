#!/usr/bin/env python3
"""Zenko native Cargo progress renderer.

Replaces external `cargo-pretty` (`nobubbles-0.1.1`) which suffers from:
  - False 100% stall (`342/343` cached crates filling the bar before final binary compile/link)
  - Hardcoded 20-row absolute cursor addressing (`CSI row;colH`) overflowing short terminals
  - Terminal DSR (`ESC[6n`) cursor-position query hangs and uncleaned frames
  - Double-invocation overhead via `RUSTC_BOOTSTRAP=1 cargo --unit-graph`

Design invariants:
  - Strictly relative cursor movement within a compact 2..5 row inline viewport
  - DEC Synchronized Output (`ESC[?2026h` .. `ESC[?2026l`) for tear-free atomic frames
  - Decoupled 12.5 FPS render loop (never freezes during silent `rustc`/`ld` link phases)
  - 60/30/10 Zenko Obsidian Vermilion (`#ea5829`) monochrome ramp (zero rainbow clutter)
  - Zero regex, zero external dependencies, deterministic ANSI/status state machines
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Callable


# ---------------------------------------------------------------------------
# Centralized constants & declarative tables (zero scattered magic strings)
# ---------------------------------------------------------------------------

FRAME_INTERVAL_SECS: float = 0.08  # 12.5 FPS smooth decoupled ticker
LINKING_HINT_SECS: float = 1.6     # Hint compile+link when a single bin target runs long
MAX_ACTIVE_ROWS: int = 3
MIN_BAR_WIDTH: int = 12
MAX_BAR_WIDTH: int = 30
COMET_RADIUS: float = 4.2

CSI_HIDE_CURSOR: str = "\x1b[?25l"
CSI_SHOW_CURSOR: str = "\x1b[?25h"
CSI_CLEAR_BELOW: str = "\x1b[J"
CSI_SYNC_BEGIN: str = "\x1b[?2026h"
CSI_SYNC_END: str = "\x1b[?2026l"
SGR_RESET: str = "\x1b[0m"
SGR_BOLD: str = "\x1b[1m"
SGR_DIM: str = "\x1b[2m"

SPINNER_FRAMES: tuple[str, ...] = (
    "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏",
)
GLYPH_ACTIVE: str = "●"
GLYPH_OK: str = "✓"
GLYPH_ERR: str = "✗"
GLYPH_WARN: str = "!"
GLYPH_BAR_FILL: str = "━"
GLYPH_BAR_TRACK: str = "─"

# Zenko 60/30/10 Obsidian Vermilion palette (background inherited = 60%)
RGB_TEXT_PRIMARY: tuple[int, int, int] = (245, 245, 246)   # #f5f5f6 (30% neutral text)
RGB_TEXT_MUTED: tuple[int, int, int] = (142, 142, 152)     # #8e8e98 (30% secondary)
RGB_TRACK: tuple[int, int, int] = (58, 58, 66)             # #3a3a42 (30% neutral track)
RGB_ACCENT: tuple[int, int, int] = (234, 88, 41)           # #ea5829 (10% vermilion)
RGB_ACCENT_PEAK: tuple[int, int, int] = (248, 134, 92)     # #f8865c (comet core)
RGB_ACCENT_MID: tuple[int, int, int] = (186, 68, 32)       # #ba4420 (comet shoulder)
RGB_ACCENT_TAIL: tuple[int, int, int] = (102, 42, 24)      # #662a18 (comet tail)

PASSTHROUGH_FLAGS: frozenset[str] = frozenset({
    "-h", "--help", "-V", "--version", "--message-format",
})
SUPPORTED_SUBCOMMANDS: frozenset[str] = frozenset({
    "build", "b", "check", "c", "test", "t",
})
TEST_SUBCOMMANDS: frozenset[str] = frozenset({"test", "t"})


class ColorMode(Enum):
    PLAIN = auto()
    ANSI16 = auto()
    TRUECOLOR = auto()


class BuildPhase(Enum):
    RESOLVING = "resolving"
    LOCKING = "waiting for lock"
    COMPILING = "compiling"
    CHECKING = "checking"
    TESTING = "running tests"
    FINALIZING = "finalizing"


class AnsiStripState(Enum):
    NORMAL = auto()
    ESC = auto()
    CSI = auto()
    OSC = auto()
    OSC_ESC = auto()


@dataclass(frozen=True)
class Theme:
    mode: ColorMode

    def fg(self, rgb: tuple[int, int, int], ansi16: str) -> str:
        match self.mode:
            case ColorMode.TRUECOLOR:
                return f"\x1b[38;2;{rgb[0]};{rgb[1]};{rgb[2]}m"
            case ColorMode.ANSI16:
                return ansi16
            case ColorMode.PLAIN:
                return ""

    def paint(self, text: str, rgb: tuple[int, int, int], ansi16: str, bold: bool = False) -> str:
        if self.mode is ColorMode.PLAIN or not text:
            return text
        prefix = f"{SGR_BOLD if bold else ''}{self.fg(rgb, ansi16)}"
        return f"{prefix}{text}{SGR_RESET}"

    def primary(self, text: str, bold: bool = False) -> str:
        return self.paint(text, RGB_TEXT_PRIMARY, "\x1b[97m", bold=bold)

    def muted(self, text: str) -> str:
        return self.paint(text, RGB_TEXT_MUTED, "\x1b[90m")

    def accent(self, text: str, bold: bool = False) -> str:
        return self.paint(text, RGB_ACCENT, "\x1b[31m", bold=bold)

    def track(self, text: str) -> str:
        return self.paint(text, RGB_TRACK, "\x1b[90m")


def detect_color_mode() -> ColorMode:
    if os.environ.get("NO_COLOR") is not None or os.environ.get("TERM") == "dumb":
        return ColorMode.PLAIN
    colorterm = os.environ.get("COLORTERM", "").lower()
    term = os.environ.get("TERM", "").lower()
    if colorterm in ("truecolor", "24bit") or any(
        tok in term for tok in ("256color", "truecolor", "kitty", "ghostty", "alacritty", "wezterm")
    ):
        return ColorMode.TRUECOLOR
    return ColorMode.ANSI16


# ---------------------------------------------------------------------------
# Deterministic byte/char parsers (zero regex)
# ---------------------------------------------------------------------------

def strip_ansi(text: str) -> str:
    """Strip ANSI CSI/OSC escape sequences via a deterministic 5-state FSM."""
    out: list[str] = []
    state = AnsiStripState.NORMAL
    for ch in text:
        match state:
            case AnsiStripState.NORMAL:
                if ch == "\x1b":
                    state = AnsiStripState.ESC
                else:
                    out.append(ch)
            case AnsiStripState.ESC:
                if ch == "[":
                    state = AnsiStripState.CSI
                elif ch == "]":
                    state = AnsiStripState.OSC
                else:
                    state = AnsiStripState.NORMAL
            case AnsiStripState.CSI:
                if "@" <= ch <= "~":
                    state = AnsiStripState.NORMAL
            case AnsiStripState.OSC:
                if ch == "\x07":
                    state = AnsiStripState.NORMAL
                elif ch == "\x1b":
                    state = AnsiStripState.OSC_ESC
            case AnsiStripState.OSC_ESC:
                state = AnsiStripState.NORMAL
    return "".join(out)


def parse_cargo_stderr_line(raw_line: str) -> tuple[str, str] | None:
    """Split a Cargo stderr status line (`   Compiling foo v0.1.0 (...)`) -> `(verb, rest)`."""
    clean = strip_ansi(raw_line).strip()
    if not clean:
        return None
    parts = clean.split(None, 1)
    verb = parts[0]
    if verb in STDERR_VERB_DISPATCH:
        return verb, (parts[1].strip() if len(parts) > 1 else "")
    return None


def parse_crate_label(rest: str) -> tuple[str, str]:
    """Parse `zenko-cli v0.0.29 (/path/...)` -> `(crate_key, display_label)`."""
    tokens = rest.split()
    if not tokens:
        return ("workspace", "workspace")
    name = tokens[0]
    version = tokens[1] if len(tokens) > 1 and tokens[1].startswith("v") else ""
    label = f"{name} {version}".strip()
    return (name, label)


def package_name_from_id(package_id: str, fallback: str) -> str:
    """Extract crate name from Cargo v2 package_id (`path+...#name@ver` or `.../<name>#ver`)."""
    if "#" in package_id:
        left, right = package_id.rsplit("#", 1)
        if "@" in right:
            return right.split("@", 1)[0]
        if "/" in left:
            return left.rsplit("/", 1)[1]
    if " " in package_id:
        return package_id.split(" ", 1)[0]
    return fallback


def format_elapsed(seconds: float) -> str:
    if seconds < 60.0:
        return f"{seconds:4.1f}s"
    mins, rem = divmod(int(seconds), 60)
    return f"{mins:d}m{rem:02d}s"


def clip_text(text: str, max_len: int) -> str:
    if max_len <= 0:
        return ""
    if len(text) <= max_len:
        return text
    if max_len == 1:
        return "…"
    return f"{text[:max_len - 1]}…"


# ---------------------------------------------------------------------------
# Live build state & declarative event handlers
# ---------------------------------------------------------------------------

@dataclass
class ActiveUnit:
    key: str
    label: str
    started_at: float


@dataclass
class BuildState:
    command_label: str
    is_test_cmd: bool
    started_at: float = field(default_factory=time.monotonic)
    phase: BuildPhase = BuildPhase.RESOLVING
    active_units: dict[str, ActiveUnit] = field(default_factory=dict)
    built_count: int = 0
    cached_count: int = 0
    last_built_label: str = ""
    max_displayed_pct: int = 0
    warning_count: int = 0
    error_count: int = 0
    diagnostics: list[str] = field(default_factory=list)
    extra_stderr: list[str] = field(default_factory=list)
    build_finished: bool = False
    build_success: bool | None = None
    passthrough_started: bool = False
    finished_summary_emitted: bool = False
    lock: threading.Lock = field(default_factory=threading.Lock)


def _on_stderr_compiling(state: BuildState, rest: str, now: float) -> None:
    key, label = parse_crate_label(rest)
    state.phase = BuildPhase.COMPILING
    state.active_units.setdefault(key, ActiveUnit(key=key, label=label, started_at=now))


def _on_stderr_checking(state: BuildState, rest: str, now: float) -> None:
    key, label = parse_crate_label(rest)
    state.phase = BuildPhase.CHECKING
    state.active_units.setdefault(key, ActiveUnit(key=key, label=label, started_at=now))


def _on_stderr_locking(state: BuildState, _rest: str, _now: float) -> None:
    state.phase = BuildPhase.LOCKING


def _on_stderr_resolving(state: BuildState, _rest: str, _now: float) -> None:
    if not state.active_units:
        state.phase = BuildPhase.RESOLVING


def _on_stderr_running(state: BuildState, _rest: str, _now: float) -> None:
    if state.is_test_cmd:
        state.phase = BuildPhase.TESTING
        state.build_finished = True
        state.active_units.clear()


def _on_stderr_finished(state: BuildState, _rest: str, _now: float) -> None:
    state.phase = BuildPhase.FINALIZING
    state.active_units.clear()


def _on_stderr_fresh(state: BuildState, _rest: str, _now: float) -> None:
    state.cached_count += 1


STDERR_VERB_DISPATCH: dict[str, Callable[[BuildState, str, float], None]] = {
    "Compiling": _on_stderr_compiling,
    "Checking": _on_stderr_checking,
    "Documenting": _on_stderr_compiling,
    "Blocking": _on_stderr_locking,
    "Downloading": _on_stderr_resolving,
    "Updating": _on_stderr_resolving,
    "Fetching": _on_stderr_resolving,
    "Locking": _on_stderr_resolving,
    "Running": _on_stderr_running,
    "Finished": _on_stderr_finished,
    "Fresh": _on_stderr_fresh,
}


def _on_json_artifact(state: BuildState, obj: dict) -> None:
    fresh = bool(obj.get("fresh", False))
    target = obj.get("target") or {}
    target_name = str(target.get("name") or "")
    pkg_name = package_name_from_id(str(obj.get("package_id") or ""), target_name)

    if fresh:
        state.cached_count += 1
        return

    state.built_count += 1
    state.last_built_label = pkg_name or target_name
    # Remove completed crate from active set (matching both hyphen and underscore variants)
    for candidate in (pkg_name, target_name, target_name.replace("_", "-")):
        if candidate and candidate in state.active_units:
            state.active_units.pop(candidate, None)


def _on_json_build_script(state: BuildState, obj: dict) -> None:
    # Cargo marks cached build-script executions without `fresh`, treat as cached unit when no active build script
    pkg_name = package_name_from_id(str(obj.get("package_id") or ""), "")
    if pkg_name in state.active_units:
        state.active_units.pop(pkg_name, None)
        state.built_count += 1
    else:
        state.cached_count += 1


def _on_json_compiler_message(state: BuildState, obj: dict) -> None:
    msg = obj.get("message") or {}
    level = str(msg.get("level") or "")
    rendered = msg.get("rendered")
    if level == "warning":
        # Ignore Cargo's aggregate summary line when counting individual warnings
        text = str(msg.get("message") or "")
        if not text.endswith("warning emitted") and not text.endswith("warnings emitted"):
            state.warning_count += 1
    elif level == "error":
        text = str(msg.get("message") or "")
        if not text.startswith("aborting due to") and not text.startswith("could not compile"):
            state.error_count += 1
    if isinstance(rendered, str) and rendered.strip():
        state.diagnostics.append(rendered)


def _on_json_build_finished(state: BuildState, obj: dict) -> None:
    state.build_finished = True
    state.build_success = bool(obj.get("success", False))
    state.active_units.clear()
    if state.build_success and state.is_test_cmd:
        state.phase = BuildPhase.TESTING
    else:
        state.phase = BuildPhase.FINALIZING


JSON_REASON_DISPATCH: dict[str, Callable[[BuildState, dict], None]] = {
    "compiler-artifact": _on_json_artifact,
    "build-script-executed": _on_json_build_script,
    "compiler-message": _on_json_compiler_message,
    "build-finished": _on_json_build_finished,
}


# ---------------------------------------------------------------------------
# Tear-free inline renderer (strictly relative cursor motion)
# ---------------------------------------------------------------------------

class InlineRenderer:
    def __init__(self, theme: Theme) -> None:
        self.theme = theme
        self.prev_lines: int = 0
        self.cursor_hidden: bool = False
        self.tick: int = 0
        self.out = sys.stdout

    def _terminal_dims(self) -> tuple[int, int]:
        size = shutil.get_terminal_size(fallback=(80, 24))
        return max(32, size.columns), max(6, size.lines)

    def _comet_rgb(self, dist: float) -> tuple[int, int, int] | None:
        if dist <= 1.1:
            return RGB_ACCENT_PEAK
        if dist <= 2.2:
            return RGB_ACCENT
        if dist <= 3.3:
            return RGB_ACCENT_MID
        if dist <= COMET_RADIUS:
            return RGB_ACCENT_TAIL
        return None

    def _paint_runs(self, cells: list[tuple[str, tuple[int, int, int], str, bool]]) -> str:
        """Group adjacent cells sharing style to minimize ANSI bytes per frame."""
        if not cells:
            return ""
        out: list[str] = []
        run_chars: list[str] = []
        cur_style: tuple[tuple[int, int, int], str, bool] | None = None

        def flush() -> None:
            if not run_chars or cur_style is None:
                return
            rgb, ansi16, bold = cur_style
            out.append(self.theme.paint("".join(run_chars), rgb, ansi16, bold=bold))
            run_chars.clear()

        for ch, rgb, ansi16, bold in cells:
            style = (rgb, ansi16, bold)
            if style != cur_style:
                flush()
                cur_style = style
            run_chars.append(ch)
        flush()
        return "".join(out)

    def _render_indeterminate_bar(self, width: int) -> str:
        # Smooth triangle-wave sweep (never wraps abruptly, never freezes!)
        period = max(2, (width - 1) * 2)
        pos_step = self.tick % period
        center = float(pos_step if pos_step < width else period - pos_step)

        if self.theme.mode is ColorMode.PLAIN:
            return "".join("=" if abs(i - center) <= 2.2 else "-" for i in range(width))

        cells: list[tuple[str, tuple[int, int, int], str, bool]] = []
        for idx in range(width):
            rgb = self._comet_rgb(abs(idx - center))
            if rgb is not None:
                cells.append((GLYPH_BAR_FILL, rgb, "\x1b[31m", rgb == RGB_ACCENT_PEAK))
            else:
                cells.append((GLYPH_BAR_TRACK, RGB_TRACK, "\x1b[90m", False))
        return self._paint_runs(cells)

    def _render_determinate_bar(self, width: int, pct: int) -> str:
        filled = max(1, min(width - 1, round(width * pct / 100.0)))
        if self.theme.mode is ColorMode.PLAIN:
            return ("=" * filled) + ("-" * (width - filled))

        # Subtle living pulse along the filled bar so it never looks frozen
        pulse_pos = (self.tick // 2) % max(1, filled)
        cells: list[tuple[str, tuple[int, int, int], str, bool]] = []
        for idx in range(filled):
            is_peak = idx == filled - 1 or idx == pulse_pos
            cells.append((GLYPH_BAR_FILL, RGB_ACCENT_PEAK if is_peak else RGB_ACCENT, "\x1b[31m", is_peak))
        for _ in range(width - filled):
            cells.append((GLYPH_BAR_TRACK, RGB_TRACK, "\x1b[90m", False))
        return self._paint_runs(cells)

    def _build_lines(self, state: BuildState, now: float) -> list[str]:
        cols, rows = self._terminal_dims()
        max_width = cols - 1
        max_rows = min(2 + MAX_ACTIVE_ROWS, max(2, rows - 1))

        spinner = SPINNER_FRAMES[self.tick % len(SPINNER_FRAMES)]
        total_elapsed = format_elapsed(max(0.0, now - state.started_at))

        # Line 1: `<spinner> cargo <args>   <elapsed>`
        cmd_budget = max(10, max_width - len(total_elapsed) - 4)
        cmd_clipped = clip_text(state.command_label, cmd_budget)
        pad_hdr = max(1, max_width - (2 + len(cmd_clipped) + len(total_elapsed)))
        header_line = (
            f"{self.theme.accent(spinner, bold=True)} "
            f"{self.theme.primary(cmd_clipped, bold=True)}"
            f"{' ' * pad_hdr}"
            f"{self.theme.muted(total_elapsed)}"
        )
        lines: list[str] = [header_line]

        # Active crate rows (only when crates are actively compiling; zero empty padding!)
        active_list = sorted(state.active_units.values(), key=lambda u: u.started_at)
        active_slot_budget = max(0, max_rows - 2)
        if active_list and active_slot_budget > 0:
            visible_count = (
                active_slot_budget
                if len(active_list) <= active_slot_budget
                else max(1, active_slot_budget - 1)
            )
            single_final_link = (
                len(active_list) == 1
                and state.built_count == 0
                and state.cached_count >= 16
                and not state.is_test_cmd
                and state.phase is BuildPhase.COMPILING
            )
            for unit in active_list[:visible_count]:
                unit_secs = max(0.0, now - unit.started_at)
                unit_elapsed = format_elapsed(unit_secs)
                suffix = (
                    " · compiling & linking"
                    if single_final_link and unit_secs >= LINKING_HINT_SECS
                    else ""
                )
                label_budget = max(8, max_width - len(unit_elapsed) - 6)
                label_text = clip_text(f"{unit.label}{suffix}", label_budget)
                pad_row = max(1, max_width - (4 + len(label_text) + len(unit_elapsed)))
                lines.append(
                    f"  {self.theme.accent(GLYPH_ACTIVE)} "
                    f"{self.theme.primary(label_text)}"
                    f"{' ' * pad_row}"
                    f"{self.theme.muted(unit_elapsed)}"
                )
            overflow = len(active_list) - visible_count
            if overflow > 0:
                more_text = clip_text(f"+{overflow} more {state.phase.value}…", max_width - 4)
                lines.append(f"    {self.theme.muted(more_text)}")

        # Bottom line: sleek progress bar + truthful status (NEVER 100% while active!)
        active_count = len(active_list)
        built = state.built_count
        cached = state.cached_count

        if built > 0 and active_count > 0:
            raw_pct = int(round(100.0 * built / (built + active_count)))
            pct = max(state.max_displayed_pct, min(95, max(5, raw_pct)))
            state.max_displayed_pct = pct
            status_parts = [f"{pct:>2d}%", f"{built}/{built + active_count}"]
            if cached > 0:
                status_parts.append(f"{cached} cached")
            status_plain = " · ".join(status_parts)
            bar_width = max(MIN_BAR_WIDTH, min(MAX_BAR_WIDTH, max_width - len(status_plain) - 5))
            bar_str = self._render_determinate_bar(bar_width, pct)
        else:
            # Indeterminate smooth vermilion comet while resolving, locking, or single-crate compiling/linking
            if (
                active_count == 1
                and built == 0
                and cached >= 16
                and not state.is_test_cmd
                and state.phase is BuildPhase.COMPILING
                and (now - active_list[0].started_at) >= LINKING_HINT_SECS
            ):
                stage_text = "linking"
            else:
                stage_text = state.phase.value
            status_parts = [stage_text]
            if built > 0:
                status_parts.append(f"{built} built")
            if cached > 0:
                status_parts.append(f"{cached} cached")
            if state.warning_count > 0:
                status_parts.append(f"{state.warning_count} warn")
            status_plain = " · ".join(status_parts)
            bar_width = max(MIN_BAR_WIDTH, min(MAX_BAR_WIDTH, max_width - len(status_plain) - 5))
            bar_str = self._render_indeterminate_bar(bar_width)

        status_clipped = clip_text(status_plain, max(4, max_width - bar_width - 4))
        lines.append(f"  {bar_str}  {self.theme.muted(status_clipped)}")
        return lines

    def render(self, state: BuildState) -> None:
        now = time.monotonic()
        with state.lock:
            lines = self._build_lines(state, now)

        buf: list[str] = [CSI_SYNC_BEGIN]
        if not self.cursor_hidden:
            buf.append(CSI_HIDE_CURSOR)
            self.cursor_hidden = True

        if self.prev_lines > 1:
            buf.append(f"\r\x1b[{self.prev_lines - 1}A")
        elif self.prev_lines == 1:
            buf.append("\r")

        buf.append(CSI_CLEAR_BELOW)
        buf.append("\r\n".join(lines))
        buf.append(CSI_SYNC_END)

        self.out.write("".join(buf))
        self.out.flush()
        self.prev_lines = len(lines)
        self.tick += 1

    def clear(self) -> None:
        buf: list[str] = [CSI_SYNC_BEGIN]
        if self.prev_lines > 1:
            buf.append(f"\r\x1b[{self.prev_lines - 1}A{CSI_CLEAR_BELOW}")
        elif self.prev_lines == 1:
            buf.append(f"\r{CSI_CLEAR_BELOW}")
        if self.cursor_hidden:
            buf.append(CSI_SHOW_CURSOR)
            self.cursor_hidden = False
        buf.append(SGR_RESET)
        buf.append(CSI_SYNC_END)
        self.out.write("".join(buf))
        self.out.flush()
        self.prev_lines = 0

    def print_summary(self, state: BuildState, success: bool, interrupted: bool = False) -> None:
        elapsed = format_elapsed(max(0.0, time.monotonic() - state.started_at)).strip()

        # Print compiler diagnostics first on failure (or after compact summary on success with warnings)
        if not success and not interrupted:
            for diag in state.diagnostics:
                sys.stderr.write(diag)
                if not diag.endswith("\n"):
                    sys.stderr.write("\n")
            for extra in state.extra_stderr:
                sys.stderr.write(f"{extra}\n")
            sys.stderr.flush()

            err_note = f" ({state.error_count} error{'s' if state.error_count != 1 else ''})" if state.error_count else ""
            self.out.write(
                f"{self.theme.accent(GLYPH_ERR, bold=True)} "
                f"{self.theme.primary(f'Build failed in {elapsed}{err_note}', bold=True)}\n"
            )
            self.out.flush()
            return

        if interrupted:
            self.out.write(
                f"{self.theme.accent(GLYPH_ERR, bold=True)} "
                f"{self.theme.muted(f'Cancelled after {elapsed}')}\n"
            )
            self.out.flush()
            return

        details: list[str] = []
        if state.built_count > 0:
            details.append(f"{state.built_count} compiled")
        if state.cached_count > 0:
            details.append(f"{state.cached_count} cached")
        if state.warning_count > 0:
            details.append(f"{state.warning_count} warning{'s' if state.warning_count != 1 else ''}")
        detail_suffix = f" ({', '.join(details)})" if details else ""

        if state.is_test_cmd:
            headline = f"Compiled tests in {elapsed}"
        elif state.built_count == 0:
            headline = f"Up to date in {elapsed}"
        else:
            target_hint = f" {state.last_built_label}" if state.last_built_label else ""
            headline = f"Built{target_hint} in {elapsed}"

        self.out.write(
            f"{self.theme.accent(GLYPH_OK, bold=True)} "
            f"{self.theme.primary(headline, bold=True)}"
            f"{self.theme.muted(detail_suffix)}\n"
        )
        self.out.flush()

        if state.diagnostics:
            for diag in state.diagnostics:
                sys.stderr.write(diag)
                if not diag.endswith("\n"):
                    sys.stderr.write("\n")
            sys.stderr.flush()


# ---------------------------------------------------------------------------
# Cargo argument builder & execution driver
# ---------------------------------------------------------------------------

def should_passthrough_raw(cargo_args: list[str]) -> bool:
    if not cargo_args or cargo_args[0] not in SUPPORTED_SUBCOMMANDS:
        return True
    before_sep = cargo_args[: cargo_args.index("--")] if "--" in cargo_args else cargo_args
    return any(
        arg in PASSTHROUGH_FLAGS or arg.startswith("--message-format=")
        for arg in before_sep
    )


def inject_cargo_flags(cargo_args: list[str]) -> list[str]:
    injected = ["--message-format=json-render-diagnostics"]
    if not any(a == "--color" or a.startswith("--color=") for a in cargo_args):
        if os.environ.get("NO_COLOR") is None:
            injected.append("--color=always")

    if "--" in cargo_args:
        sep = cargo_args.index("--")
        return ["cargo", *cargo_args[:sep], *injected, *cargo_args[sep:]]
    return ["cargo", *cargo_args, *injected]


def run_pretty_cargo(cargo_args: list[str]) -> int:
    if should_passthrough_raw(cargo_args) or not sys.stdout.isatty():
        os.execvp("cargo", ["cargo", *cargo_args])

    is_test_cmd = cargo_args[0] in TEST_SUBCOMMANDS
    theme = Theme(detect_color_mode())
    renderer = InlineRenderer(theme)
    state = BuildState(
        command_label=f"cargo {' '.join(cargo_args)}",
        is_test_cmd=is_test_cmd,
    )

    cmd = inject_cargo_flags(cargo_args)
    env = os.environ.copy()
    env.setdefault("CARGO_TERM_PROGRESS_WHEN", "never")

    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )

    interrupted = threading.Event()
    received_sig: list[int] = []
    wake_event = threading.Event()
    passthrough_gate = threading.Event()
    pending_test_out: list[tuple[bool, str]] = []

    def handle_signal(signum: int, _frame: object) -> None:
        received_sig.append(signum)
        interrupted.set()
        wake_event.set()
        if proc.poll() is None:
            try:
                proc.send_signal(signum)
            except ProcessLookupError:
                pass

    prev_sigint = signal.signal(signal.SIGINT, handle_signal)
    prev_sigterm = signal.signal(signal.SIGTERM, handle_signal)

    def read_stdout() -> None:
        assert proc.stdout is not None
        for raw_line in proc.stdout:
            if passthrough_gate.is_set():
                sys.stdout.write(raw_line)
                sys.stdout.flush()
                continue
            stripped = raw_line.strip()
            if stripped.startswith("{") and stripped.endswith("}"):
                try:
                    obj = json.loads(stripped)
                except json.JSONDecodeError:
                    obj = None
                if isinstance(obj, dict) and "reason" in obj:
                    reason = str(obj["reason"])
                    handler = JSON_REASON_DISPATCH.get(reason)
                    if handler is not None:
                        with state.lock:
                            handler(state, obj)
                    if reason == "build-finished":
                        wake_event.set()
                    continue
            # Non-JSON stdout line (e.g., `libtest` output during `cargo test`)
            with state.lock:
                state.passthrough_started = True
                pending_test_out.append((False, raw_line))
            wake_event.set()
        wake_event.set()

    def read_stderr() -> None:
        assert proc.stderr is not None
        for raw_line in proc.stderr:
            if passthrough_gate.is_set():
                sys.stderr.write(raw_line)
                sys.stderr.flush()
                continue
            now = time.monotonic()
            parsed = parse_cargo_stderr_line(raw_line)
            if parsed is not None:
                verb, rest = parsed
                with state.lock:
                    STDERR_VERB_DISPATCH[verb](state, rest, now)
                    if is_test_cmd and verb == "Running":
                        state.passthrough_started = True
                        pending_test_out.append((True, raw_line))
                        wake_event.set()
                continue
            cleaned = raw_line.rstrip("\r\n")
            if cleaned.strip():
                with state.lock:
                    state.extra_stderr.append(cleaned)
        wake_event.set()

    def flush_pending_test_output() -> None:
        with state.lock:
            buffered = list(pending_test_out)
            pending_test_out.clear()
        for is_err, line in buffered:
            target = sys.stderr if is_err else sys.stdout
            target.write(line)
            target.flush()

    t_out = threading.Thread(target=read_stdout, daemon=True)
    t_err = threading.Thread(target=read_stderr, daemon=True)
    t_out.start()
    t_err.start()

    try:
        while proc.poll() is None and not interrupted.is_set():
            with state.lock:
                enter_passthrough = is_test_cmd and (
                    state.passthrough_started or (state.build_finished and state.build_success)
                )
            if enter_passthrough and not passthrough_gate.is_set():
                renderer.clear()
                with state.lock:
                    renderer.print_summary(state, success=True)
                    state.finished_summary_emitted = True
                    buffered = list(pending_test_out)
                    pending_test_out.clear()
                    for is_err, line in buffered:
                        target = sys.stderr if is_err else sys.stdout
                        target.write(line)
                        target.flush()
                    passthrough_gate.set()
                # Wait for child to finish streaming tests
                while proc.poll() is None and not interrupted.is_set():
                    wake_event.clear()
                    wake_event.wait(timeout=FRAME_INTERVAL_SECS)
                break

            renderer.render(state)
            wake_event.clear()
            wake_event.wait(timeout=FRAME_INTERVAL_SECS)

        rc = proc.wait()
        t_out.join(timeout=2.0)
        t_err.join(timeout=2.0)
    finally:
        signal.signal(signal.SIGINT, prev_sigint)
        signal.signal(signal.SIGTERM, prev_sigterm)
        if not passthrough_gate.is_set():
            renderer.clear()

    if interrupted.is_set():
        renderer.print_summary(state, success=False, interrupted=True)
        sig = received_sig[0] if received_sig else signal.SIGINT
        return 128 + int(sig)

    if passthrough_gate.is_set():
        flush_pending_test_output()
        return rc

    # Fast test runs may complete within one frame before entering passthrough; emit summary + test output cleanly.
    if is_test_cmd and pending_test_out:
        if not state.finished_summary_emitted:
            renderer.print_summary(state, success=(state.build_success is not False))
            state.finished_summary_emitted = True
        flush_pending_test_output()
        return rc

    success = rc == 0 and (state.build_success is not False)
    if not state.finished_summary_emitted:
        renderer.print_summary(state, success=success)
    return rc


def main() -> int:
    return run_pretty_cargo(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())
