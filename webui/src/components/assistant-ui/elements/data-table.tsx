"use client";

import { useEffect, useState, type ComponentProps } from "react";
import { cn } from "@/lib/utils";
import {
  Table as EaseTable,
  TableBody as EaseTableBody,
  TableCell as EaseTableCell,
  TableHead as EaseTableHead,
  TableHeader as EaseTableHeader,
  TableRow as EaseTableRow,
} from "@/components/easeui/table";
import { safeHref } from "../utils/href";
import { clamp } from "../utils/range";
import { useIsomorphicLayoutEffect } from "../utils/useIsomorphicLayoutEffect";
import { field, mono, paper } from "./surfaces";

export type DataTableValue =
  | string
  | number
  | boolean
  | null
  | undefined
  | readonly (string | number)[];

export type DataTableRow = Readonly<Record<string, DataTableValue>>;

export type DataTableFormat =
  | { kind: "text" }
  | {
      kind: "number";
      decimals?: number | undefined;
      compact?: boolean | undefined;
      unit?: string | undefined;
    }
  | {
      kind: "currency";
      currency: string;
      decimals?: number | undefined;
      compact?: boolean | undefined;
    }
  | {
      kind: "percent";
      decimals?: number | undefined;
      basis?: "fraction" | "unit" | undefined;
    }
  | {
      kind: "delta";
      decimals?: number | undefined;
      unit?: string | undefined;
      upIsGood?: boolean | undefined;
    }
  | { kind: "date"; style?: "date" | "datetime" | "relative" | undefined }
  | {
      kind: "boolean";
      trueLabel?: string | undefined;
      falseLabel?: string | undefined;
    }
  | { kind: "link"; hrefKey?: string | undefined }
  | {
      kind: "badge";
      tones?:
        | Readonly<Record<string, "neutral" | "success" | "warning" | "danger">>
        | undefined;
    }
  | { kind: "list"; max?: number | undefined };

export interface DataTableColumn {
  key: string;
  label: string;
  format?: DataTableFormat | undefined;
  align?: "start" | "end" | undefined;
  sortable?: boolean | undefined;
  priority?: "primary" | "secondary" | undefined;
  width?: string | undefined;
}

export interface DataTableSort {
  key: string;
  direction: "asc" | "desc";
}

export interface DataTableProps extends Omit<
  ComponentProps<"div">,
  "children"
> {
  columns: readonly DataTableColumn[];
  rows: readonly DataTableRow[];
  rowKey?: string | undefined;
  caption?: string | undefined;
  defaultSort?: DataTableSort | undefined;
  sort?: DataTableSort | null | undefined;
  onSortChange?: ((sort: DataTableSort | null) => void) | undefined;
  locale?: string | undefined;
  emptyMessage?: string | undefined;
  relativeTo?: number | undefined;
  tableStyle?:
    | "borderless"
    | "minimal"
    | "striped"
    | "bordered"
    | "easeui_table"
    | undefined;
}

const isAsciiDigit = (code: number) => code >= 48 && code <= 57;

const isDateOnlyString = (s: string): boolean =>
  s.length === 10 &&
  s[4] === "-" &&
  s[7] === "-" &&
  isAsciiDigit(s.charCodeAt(0)) &&
  isAsciiDigit(s.charCodeAt(1)) &&
  isAsciiDigit(s.charCodeAt(2)) &&
  isAsciiDigit(s.charCodeAt(3)) &&
  isAsciiDigit(s.charCodeAt(5)) &&
  isAsciiDigit(s.charCodeAt(6)) &&
  isAsciiDigit(s.charCodeAt(8)) &&
  isAsciiDigit(s.charCodeAt(9));

const isIsoDateString = (s: string): boolean =>
  s.length >= 10 &&
  isDateOnlyString(s.slice(0, 10)) &&
  (s.length === 10 || s[10] === "T");

const BADGE_TONE_CLASSES: Readonly<Record<string, string>> = {
  neutral: cn(field, "text-foreground/60"),
  success: "bg-[color-mix(in_srgb,var(--success)_14%,transparent)] text-[var(--success)]",
  warning: "bg-[color-mix(in_srgb,var(--warning)_14%,transparent)] text-[var(--warning)]",
  danger: "bg-[color-mix(in_srgb,var(--destructive)_14%,transparent)] text-[var(--destructive)]",
};

const isEmpty = (value: DataTableValue) =>
  value === null || value === undefined;

const textValue = (value: Exclude<DataTableValue, null | undefined>) =>
  Array.isArray(value) ? value.join(", ") : String(value);

const isCalendarDate = (value: string) => {
  const [year, month, day] = value.slice(0, 10).split("-").map(Number);
  if (year === undefined || month === undefined || day === undefined) {
    return false;
  }
  if (month < 1 || month > 12 || day < 1) return false;
  const daysInMonth =
    month === 2
      ? year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0)
        ? 29
        : 28
      : [4, 6, 9, 11].includes(month)
        ? 30
        : 31;
  return day <= daysInMonth;
};

const asDate = (value: DataTableValue) => {
  const timestamp =
    typeof value === "number"
      ? value
      : typeof value === "string" &&
          isIsoDateString(value) &&
          isCalendarDate(value)
        ? Date.parse(value)
        : Number.NaN;
  return Number.isNaN(timestamp) ? undefined : new Date(timestamp);
};

const fractionDigits = (decimals: number | undefined) =>
  decimals === undefined || !Number.isFinite(decimals)
    ? {}
    : {
        minimumFractionDigits: Math.floor(clamp(decimals, 0, 20)),
        maximumFractionDigits: Math.floor(clamp(decimals, 0, 20)),
      };

const numberOptions = (decimals: number | undefined, compact: boolean) => ({
  notation: compact ? ("compact" as const) : ("standard" as const),
  ...fractionDigits(decimals),
});

/**
 * Formats with Intl, falling back to the raw value: a model can send a
 * currency code or locale that Intl rejects by throwing.
 */
const formatted = (
  value: Exclude<DataTableValue, null | undefined>,
  format: () => string,
) => {
  try {
    return format();
  } catch {
    return textValue(value);
  }
};

const numericValue = (value: DataTableValue) =>
  typeof value === "number" && !Number.isNaN(value) ? value : undefined;

const relativeDate = (date: Date, locale: string, relativeTo: number) => {
  const seconds = Math.round((date.getTime() - relativeTo) / 1000);
  const units: readonly [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31_536_000],
    ["month", 2_592_000],
    ["week", 604_800],
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
    ["second", 1],
  ];
  const [unit, amount] = units.find(
    ([, size]) => Math.abs(seconds) >= size,
  ) ?? ["second", 1];
  return new Intl.RelativeTimeFormat(locale, { numeric: "auto" }).format(
    Math.round(seconds / amount),
    unit,
  );
};

const relativeRefreshDelay = (
  columns: readonly DataTableColumn[],
  rows: readonly DataTableRow[],
  now: number,
) => {
  const relativeKeys = columns
    .filter(
      (column) =>
        column.format?.kind === "date" && column.format.style === "relative",
    )
    .map((column) => column.key);
  const freshestAge = rows.reduce((minimum, row) => {
    for (const key of relativeKeys) {
      const date = asDate(row[key]);
      if (date) minimum = Math.min(minimum, Math.abs(now - date.getTime()));
    }
    return minimum;
  }, Number.POSITIVE_INFINITY);

  if (freshestAge < 60_000) return 1_000;
  if (freshestAge < 3_600_000) return 30_000;
  if (freshestAge < 86_400_000) return 60_000;
  if (freshestAge < 604_800_000) return 3_600_000;
  return 86_400_000;
};

const withUnit = (text: string, unit: string | undefined) =>
  unit === undefined || unit === ""
    ? text
    : /^[%‰°]/.test(unit)
      ? `${text}${unit}`
      : `${text} ${unit}`;

const EmptyValue = () => <span className="text-muted-foreground">none</span>;

function Value({
  column,
  row,
  locale,
  relativeTo,
}: {
  column: DataTableColumn;
  row: DataTableRow;
  locale: string;
  relativeTo: number | undefined;
}) {
  const value = row[column.key];
  if (isEmpty(value)) return <EmptyValue />;

  const format = column.format ?? { kind: "text" as const };
  switch (format.kind) {
    case "number": {
      const number = numericValue(value);
      if (number === undefined) return <>{textValue(value)}</>;
      const content = formatted(value, () =>
        new Intl.NumberFormat(
          locale,
          numberOptions(format.decimals, format.compact === true),
        ).format(number),
      );
      return <>{withUnit(content, format.unit)}</>;
    }
    case "currency": {
      const number = numericValue(value);
      if (number === undefined) return <>{textValue(value)}</>;
      return (
        <>
          {formatted(value, () =>
            new Intl.NumberFormat(locale, {
              style: "currency",
              currency: format.currency,
              ...numberOptions(format.decimals, format.compact === true),
            }).format(number),
          )}
        </>
      );
    }
    case "percent": {
      const number = numericValue(value);
      if (number === undefined) return <>{textValue(value)}</>;
      return (
        <>
          {formatted(value, () =>
            new Intl.NumberFormat(locale, {
              style: "percent",
              ...fractionDigits(format.decimals),
            }).format(format.basis === "unit" ? number / 100 : number),
          )}
        </>
      );
    }
    case "delta": {
      const number = numericValue(value);
      if (number === undefined) return <>{textValue(value)}</>;
      const direction = number > 0 ? "↑" : number < 0 ? "↓" : "→";
      const good = number === 0 || number > 0 === (format.upIsGood ?? true);
      const content = formatted(value, () =>
        new Intl.NumberFormat(locale, {
          signDisplay: "never",
          ...fractionDigits(format.decimals),
        }).format(number),
      );
      return (
        <span
          className={cn(
            number === 0
              ? "text-muted-foreground"
              : good
                ? "text-[var(--success)]"
                : "text-[var(--destructive)]",
          )}
        >
          {direction} {withUnit(content, format.unit)}
        </span>
      );
    }
    case "date": {
      const date = asDate(value);
      if (!date) return <>{textValue(value)}</>;
      const style = format.style ?? "date";
      const content = formatted(value, () =>
        style === "relative" && relativeTo !== undefined
          ? relativeDate(date, locale, relativeTo)
          : new Intl.DateTimeFormat(
              locale,
              style === "datetime"
                ? { dateStyle: "medium", timeStyle: "short" }
                : {
                    dateStyle: "medium",
                    ...((style === "relative" && relativeTo === undefined) ||
                    (typeof value === "string" && isDateOnlyString(value))
                      ? { timeZone: "UTC" }
                      : {}),
                  },
            ).format(date),
      );
      return <span title={date.toISOString()}>{content}</span>;
    }
    case "boolean": {
      if (typeof value !== "boolean") return <>{textValue(value)}</>;
      return (
        <>{value ? (format.trueLabel ?? "Yes") : (format.falseLabel ?? "No")}</>
      );
    }
    case "link": {
      const hrefValue = format.hrefKey ? row[format.hrefKey] : value;
      const href = safeHref(
        typeof hrefValue === "string" ? hrefValue : undefined,
      );
      if (!href) return <>{textValue(value)}</>;
      const external = href.startsWith("http:") || href.startsWith("https:");
      return external ? (
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="underline underline-offset-2"
        >
          {textValue(value)}
          <span className="sr-only"> (opens in a new tab)</span>
        </a>
      ) : (
        <a href={href} className="underline underline-offset-2">
          {textValue(value)}
        </a>
      );
    }
    case "badge": {
      const tone = format.tones?.[textValue(value)] ?? "neutral";
      return (
        <span
          className={cn(
            "inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
            BADGE_TONE_CLASSES[tone] ?? BADGE_TONE_CLASSES.neutral,
          )}
        >
          {textValue(value)}
        </span>
      );
    }
    case "list": {
      if (!Array.isArray(value)) return <>{textValue(value)}</>;
      const max = Math.max(0, Math.floor(format.max ?? value.length));
      const shown = value.slice(0, max);
      return (
        <span className="flex flex-wrap gap-1">
          {shown.map((item, index) => (
            <span
              key={index}
              className={cn(
                field,
                "text-foreground/60 rounded-md px-1.5 py-0.5 text-xs",
              )}
            >
              {item}
            </span>
          ))}
          {value.length > shown.length ? (
            <span className="text-muted-foreground px-1.5 py-0.5 text-xs">
              +{value.length - shown.length}
            </span>
          ) : null}
        </span>
      );
    }
    default:
      return <>{textValue(value)}</>;
  }
}

const isNumeric = (format: DataTableFormat | undefined) =>
  format?.kind === "number" ||
  format?.kind === "currency" ||
  format?.kind === "percent" ||
  format?.kind === "delta";

const compareValues = (
  a: DataTableValue,
  b: DataTableValue,
  column: DataTableColumn,
  collator: Intl.Collator,
) => {
  if (isEmpty(a)) return isEmpty(b) ? 0 : 1;
  if (isEmpty(b)) return -1;
  if (column.format?.kind === "date") {
    const first = asDate(a);
    const second = asDate(b);
    if (first && second) return first.getTime() - second.getTime();
  }
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "boolean" && typeof b === "boolean") {
    return Number(a) - Number(b);
  }
  if (Array.isArray(a) && Array.isArray(b)) return a.length - b.length;
  return collator.compare(textValue(a), textValue(b));
};

const collatorFor = (locale: string) => {
  try {
    return new Intl.Collator(locale, { numeric: true, sensitivity: "base" });
  } catch {
    return new Intl.Collator("en-US", { numeric: true, sensitivity: "base" });
  }
};

const rowIdentifier = (
  row: DataTableRow,
  key: string | undefined,
  index: number,
) => {
  const value = key === undefined ? undefined : row[key];
  return value === null || value === undefined ? index : textValue(value);
};

export function DataTable({
  columns,
  rows,
  rowKey,
  caption,
  defaultSort,
  sort,
  onSortChange,
  locale = "en-US",
  emptyMessage = "No rows",
  relativeTo,
  tableStyle,
  className,
  ...props
}: DataTableProps) {
  const [internalSort, setInternalSort] = useState<DataTableSort | null>(
    defaultSort ?? null,
  );
  const [announcement, setAnnouncement] = useState("");
  const [colWidths, setColWidths] = useState<Record<string, number>>({});
  const [docTableStyle, setDocTableStyle] = useState<string>(() =>
    typeof document !== "undefined"
      ? document.documentElement.dataset.tableStyle || "borderless"
      : "borderless",
  );
  useEffect(() => {
    if (tableStyle !== undefined || typeof document === "undefined") return;
    const root = document.documentElement;
    const sync = () => setDocTableStyle(root.dataset.tableStyle || "borderless");
    sync();
    const obs = new MutationObserver(sync);
    obs.observe(root, { attributes: true, attributeFilter: ["data-table-style"] });
    return () => obs.disconnect();
  }, [tableStyle]);
  const effectiveTableStyle = tableStyle ?? docTableStyle;
  const activeSort = sort === undefined ? internalSort : sort;
  const hasRelativeDates = columns.some(
    (column) =>
      column.format?.kind === "date" && column.format.style === "relative",
  );
  const [now, setNow] = useState<number>();
  useIsomorphicLayoutEffect(() => {
    if (relativeTo === undefined && hasRelativeDates) setNow(Date.now());
  }, [columns, rows, relativeTo, hasRelativeDates]);
  const relativeTime = relativeTo ?? now;
  const refreshDelay =
    relativeTo === undefined && hasRelativeDates && relativeTime !== undefined
      ? relativeRefreshDelay(columns, rows, relativeTime)
      : undefined;

  useEffect(() => {
    if (refreshDelay === undefined) return;
    const timeout = window.setTimeout(() => setNow(Date.now()), refreshDelay);
    return () => window.clearTimeout(timeout);
  }, [refreshDelay, relativeTime]);
  const sortColumn = columns.find((column) => column.key === activeSort?.key);
  const collator = collatorFor(locale);
  const indexedRows = rows.map((row, index) => ({ row, index }));
  const sortedRows =
    activeSort && sortColumn
      ? indexedRows.sort((a, b) => {
          const result = compareValues(
            a.row[sortColumn.key],
            b.row[sortColumn.key],
            sortColumn,
            collator,
          );
          if (result === 0) return a.index - b.index;
          if (
            isEmpty(a.row[sortColumn.key]) ||
            isEmpty(b.row[sortColumn.key])
          ) {
            return result;
          }
          return activeSort.direction === "asc" ? result : -result;
        })
      : indexedRows;
  const primaryColumn =
    columns.find((column) => column.priority === "primary") ?? columns[0];

  const changeSort = (column: DataTableColumn) => {
    const next =
      activeSort?.key !== column.key
        ? { key: column.key, direction: "asc" as const }
        : activeSort.direction === "asc"
          ? { key: column.key, direction: "desc" as const }
          : null;
    if (sort === undefined) setInternalSort(next);
    setAnnouncement(
      next
        ? `Sorted by ${column.label}, ${next.direction === "asc" ? "ascending" : "descending"}`
        : "Sorting cleared",
    );
    onSortChange?.(next);
  };

  const cellClass = (column: DataTableColumn) =>
    cn(
      "text-foreground/80 px-4 py-2.5 align-top text-[13px] leading-5",
      (column.align ?? (isNumeric(column.format) ? "end" : "start")) ===
        "end" && "text-end tabular-nums",
    );

  if (effectiveTableStyle === "easeui_table") {
    return (
      <div
        data-slot="data-table"
        data-variant="easeui_table"
        className={cn("@container w-full", className)}
        {...props}
      >
        <div className="hidden @sm:block">
          <EaseTable>
            {caption ? <caption className="sr-only">{caption}</caption> : null}
            <EaseTableHeader>
              <tr>
                {columns.map((column) => {
                  const direction =
                    activeSort?.key === column.key
                      ? activeSort.direction
                      : false;
                  const sortable = column.sortable !== false;
                  const alignEnd =
                    (column.align ??
                      (isNumeric(column.format) ? "end" : "start")) === "end";
                  const widthStyle = colWidths[column.key]
                    ? { width: `${Math.round(colWidths[column.key])}px` }
                    : column.width
                      ? { width: column.width }
                      : undefined;
                  return (
                    <EaseTableHead
                      key={column.key}
                      scope="col"
                      sorted={direction}
                      onSort={sortable ? () => changeSort(column) : undefined}
                      onResize={(width) =>
                        setColWidths((prev) => ({ ...prev, [column.key]: width }))
                      }
                      style={widthStyle}
                      className={cn(alignEnd && "text-right")}
                    >
                      {column.label}
                    </EaseTableHead>
                  );
                })}
              </tr>
            </EaseTableHeader>
            <EaseTableBody>
              {sortedRows.length === 0 ? (
                <EaseTableRow>
                  <EaseTableCell
                    colSpan={Math.max(columns.length, 1)}
                    className="py-6 text-center text-[13px] text-muted-foreground"
                  >
                    <span role="status">{emptyMessage}</span>
                  </EaseTableCell>
                </EaseTableRow>
              ) : (
                sortedRows.map(({ row, index }) => (
                  <EaseTableRow key={rowIdentifier(row, rowKey, index)}>
                    {columns.map((column) => {
                      const alignEnd =
                        (column.align ??
                          (isNumeric(column.format) ? "end" : "start")) === "end";
                      return (
                        <EaseTableCell
                          key={column.key}
                          emphasis={column.key === primaryColumn?.key}
                          className={cn(
                            "text-[13px]",
                            alignEnd && "text-right tabular-nums",
                          )}
                        >
                          <Value
                            column={column}
                            row={row}
                            locale={locale}
                            relativeTo={relativeTime}
                          />
                        </EaseTableCell>
                      );
                    })}
                  </EaseTableRow>
                ))
              )}
            </EaseTableBody>
          </EaseTable>
        </div>
        <div role="list" className="space-y-2 rounded-2xl border border-[var(--border)] bg-[var(--surface)]/70 p-2.5 @sm:hidden">
          {sortedRows.length === 0 ? (
            <div role="status" className="px-2 py-3 text-center text-[13px] text-muted-foreground">
              {emptyMessage}
            </div>
          ) : (
            sortedRows.map(({ row, index }) => (
              <div
                key={rowIdentifier(row, rowKey, index)}
                role="listitem"
                className="rounded-xl bg-foreground/[0.025] px-3 py-2"
              >
                {primaryColumn ? (
                  <div className="text-[13px] font-medium leading-5 text-foreground/90">
                    <Value
                      column={primaryColumn}
                      row={row}
                      locale={locale}
                      relativeTo={relativeTime}
                    />
                  </div>
                ) : null}
                {columns.length > 1 ? (
                  <dl className="mt-1.5 grid grid-cols-1 gap-x-4 gap-y-1 text-[12px] leading-4 @xs:grid-cols-2">
                    {columns
                      .filter((column) => column.key !== primaryColumn?.key)
                      .map((column) => (
                        <div
                          key={column.key}
                          className="flex min-w-0 items-baseline justify-between gap-2"
                        >
                          <dt className="truncate text-muted-foreground">
                            {column.label}
                          </dt>
                          <dd className="shrink-0 text-end tabular-nums text-foreground/75">
                            <Value
                              column={column}
                              row={row}
                              locale={locale}
                              relativeTo={relativeTime}
                            />
                          </dd>
                        </div>
                      ))}
                  </dl>
                ) : null}
              </div>
            ))
          )}
        </div>
        <p aria-live="polite" className="sr-only">
          {announcement}
        </p>
      </div>
    );
  }

  return (
    <div
      data-slot="data-table"
      className={cn(
        paper,
        "zenko-data-table @container w-full overflow-x-auto rounded-2xl",
        className,
      )}
      {...props}
    >
      <table className="hidden w-full border-collapse @sm:table">
        {caption ? <caption className="sr-only">{caption}</caption> : null}
        <thead className="zenko-dt-head border-foreground/10 border-b">
          <tr>
            {columns.map((column) => {
              const direction =
                activeSort?.key === column.key
                  ? activeSort.direction
                  : undefined;
              const sortable = column.sortable !== false;
              return (
                <th
                  key={column.key}
                  scope="col"
                  {...(direction
                    ? {
                        "aria-sort":
                          direction === "asc" ? "ascending" : "descending",
                      }
                    : {})}
                  style={column.width ? { width: column.width } : undefined}
                  className={cn(
                    mono,
                    "text-muted-foreground px-4 py-3 font-medium",
                    (column.align ??
                      (isNumeric(column.format) ? "end" : "start")) === "end"
                      ? "text-end"
                      : "text-start",
                  )}
                >
                  {sortable ? (
                    <button
                      type="button"
                      onClick={() => changeSort(column)}
                      aria-label={`Sort by ${column.label}`}
                      className="focus-visible:ring-foreground/20 -mx-1 rounded px-1 outline-none focus-visible:ring-1"
                    >
                      {column.label}
                      {direction ? (
                        <span aria-hidden className="ms-1">
                          {direction === "asc" ? "↑" : "↓"}
                        </span>
                      ) : null}
                    </button>
                  ) : (
                    column.label
                  )}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {sortedRows.length === 0 ? (
            <tr>
              <td
                colSpan={Math.max(columns.length, 1)}
                className="text-muted-foreground px-4 py-6 text-center text-[13px]"
              >
                <span role="status">{emptyMessage}</span>
              </td>
            </tr>
          ) : (
            sortedRows.map(({ row, index }) => (
              <tr
                key={rowIdentifier(row, rowKey, index)}
                className="hover:bg-foreground/[0.025] transition-colors motion-reduce:transition-none"
              >
                {columns.map((column) => (
                  <td key={column.key} className={cellClass(column)}>
                    <Value
                      column={column}
                      row={row}
                      locale={locale}
                      relativeTo={relativeTime}
                    />
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
      <div role="list" className="space-y-2 p-2.5 @sm:hidden">
        {sortedRows.length === 0 ? (
          <div
            role="status"
            className="text-muted-foreground px-2 py-3 text-center text-[13px]"
          >
            {emptyMessage}
          </div>
        ) : (
          sortedRows.map(({ row, index }) => (
            <div
              key={rowIdentifier(row, rowKey, index)}
              role="listitem"
              className="bg-foreground/[0.025] hover:bg-foreground/[0.04] rounded-xl px-3 py-2 transition-colors motion-reduce:transition-none"
            >
              {primaryColumn ? (
                <div className="text-foreground/90 text-[13px] leading-5 font-medium">
                  <Value
                    column={primaryColumn}
                    row={row}
                    locale={locale}
                    relativeTo={relativeTime}
                  />
                </div>
              ) : null}
              {columns.length > 1 ? (
                <dl className="mt-1.5 grid grid-cols-1 gap-x-4 gap-y-1 text-[12px] leading-4 @xs:grid-cols-2">
                  {columns
                    .filter((column) => column.key !== primaryColumn?.key)
                    .map((column) => (
                      <div
                        key={column.key}
                        className="flex min-w-0 items-baseline justify-between gap-2"
                      >
                        <dt className="text-muted-foreground truncate">
                          {column.label}
                        </dt>
                        <dd className="text-foreground/75 shrink-0 text-end tabular-nums">
                          <Value
                            column={column}
                            row={row}
                            locale={locale}
                            relativeTo={relativeTime}
                          />
                        </dd>
                      </div>
                    ))}
                </dl>
              ) : null}
            </div>
          ))
        )}
      </div>
      <p aria-live="polite" className="sr-only">
        {announcement}
      </p>
    </div>
  );
}
