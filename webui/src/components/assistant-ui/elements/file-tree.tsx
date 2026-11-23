"use client";

import type { ComponentProps } from "react";
import { ChevronDownIcon, FileIcon, FolderIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { mono, paper } from "./surfaces";
import { take } from "../utils/range";

export interface FileTreeNode {
  path: string;
  name: string;
  depth: number;
  kind: "folder" | "file";
  additions?: number;
  deletions?: number;
}

export function FileTree({
  nodes,
  visibleCount,
  totalAdditions,
  totalDeletions,
  title,
  className,
  ...props
}: Omit<
  ComponentProps<"div">,
  "children" | "nodes" | "visibleCount" | "totalAdditions" | "totalDeletions" | "title"
> & {
  nodes: readonly FileTreeNode[];
  visibleCount: number;
  totalAdditions: number;
  totalDeletions: number;
  title?: string;
}) {
  const files = nodes.filter((node) => node.kind === "file").length;
  const hasStats = totalAdditions > 0 || totalDeletions > 0;
  const heading =
    title ?? (hasStats ? `${files} files changed` : `${files} ${files === 1 ? "file" : "files"}`);

  return (
    <div
      data-slot="file-tree"
      className={cn(
        paper,
        "flex w-full flex-col gap-2 rounded-2xl p-3.5",
        className,
      )}
      {...props}
    >
      <div className="flex items-baseline justify-between px-1">
        <span className="text-[13px] font-medium text-[var(--fg)]">{heading}</span>
        {hasStats ? (
          <span className={cn(mono, "tabular-nums")}>
            <span className="text-[var(--success)]">+{totalAdditions}</span>{" "}
            <span className="text-[var(--destructive)]">−{totalDeletions}</span>
          </span>
        ) : (
          <span className={cn(mono, "text-muted-foreground tabular-nums")}>
            {nodes.length} entries
          </span>
        )}
      </div>

      <div className="flex flex-col">
        {take(nodes, visibleCount).map((node) => (
          <div
            key={node.path}
            className="fade-in slide-in-from-left-1 animate-in fill-mode-both hover:bg-foreground/[0.03] flex items-center gap-2 rounded-lg px-1 py-1 text-[13px] transition-colors duration-300"
            style={{ paddingInlineStart: `${0.25 + node.depth * 0.85}rem` }}
          >
            {node.kind === "folder" ? (
              <>
                <ChevronDownIcon className="text-muted-foreground size-3 shrink-0" />
                <FolderIcon className="text-muted-foreground size-3.5 shrink-0" />
                <span className="text-foreground/60 min-w-0 flex-1 truncate">
                  {node.name}
                </span>
              </>
            ) : (
              <>
                <FileIcon className="text-muted-foreground ms-3 size-3.5 shrink-0" />
                <span className="text-foreground/85 min-w-0 flex-1 truncate">
                  {node.name}
                </span>
                <span className={cn(mono, "shrink-0 tabular-nums")}>
                  {node.additions ? (
                    <span className="text-emerald-600 dark:text-emerald-400">
                      +{node.additions}
                    </span>
                  ) : null}{" "}
                  {node.deletions ? (
                    <span className="text-red-600 dark:text-red-400">
                      −{node.deletions}
                    </span>
                  ) : null}
                </span>
              </>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
