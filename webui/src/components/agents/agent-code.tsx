"use client";
// Lightweight zero-WASM deterministic code renderer preserving BeUI's AgentCode API
// while strictly honoring Zenko's active 60/30/10 theme variables and lean binary footprint.

import { Fragment } from "react";
import { cn } from "@/lib/utils";

export type AgentCodeLanguage =
  | "bash"
  | "diff"
  | "json"
  | "text"
  | "tsx"
  | "typescript";

export interface AgentCodeToken {
  content: string;
  offset: number;
  tone?: "default" | "accent" | "muted" | "success" | "destructive";
}

export type AgentCodeTokenLines = AgentCodeToken[][];

export interface AgentCodeProps {
  code: string;
  language?: AgentCodeLanguage;
  className?: string;
}

export interface AgentCodeLineProps {
  code: string;
  tokens?: AgentCodeToken[];
  className?: string;
}

const TONE_STYLE: Record<NonNullable<AgentCodeToken["tone"]>, string | undefined> = {
  default: undefined,
  accent: "var(--accent)",
  muted: "var(--muted)",
  success: "var(--success)",
  destructive: "var(--destructive)",
};

const KEYWORDS = new Set([
  "async", "await", "const", "let", "var", "function", "return", "export",
  "import", "from", "type", "interface", "if", "else", "for", "while",
  "match", "fn", "pub", "struct", "enum", "impl", "use", "true", "false",
  "null", "cargo", "git", "npm", "bun", "cd", "ls", "rg", "grep",
]);

function isWordStart(ch: string): boolean {
  const c = ch.charCodeAt(0);
  return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 95;
}

function isWordCont(ch: string): boolean {
  const c = ch.charCodeAt(0);
  return isWordStart(ch) || (c >= 48 && c <= 57) || c === 45;
}

function tokenizeLine(line: string, language: AgentCodeLanguage): AgentCodeToken[] {
  if (language === "diff") {
    if (line.startsWith("+") && !line.startsWith("+++")) {
      return [{ content: line, offset: 0, tone: "success" }];
    }
    if (line.startsWith("-") && !line.startsWith("---")) {
      return [{ content: line, offset: 0, tone: "destructive" }];
    }
    if (line.startsWith("@@")) {
      return [{ content: line, offset: 0, tone: "accent" }];
    }
    return [{ content: line, offset: 0, tone: "muted" }];
  }

  if (language === "text") {
    return [{ content: line, offset: 0, tone: "default" }];
  }

  const tokens: AgentCodeToken[] = [];
  let i = 0;
  const n = line.length;

  while (i < n) {
    const ch = line[i];

    // Comments (# for bash, // for ts/tsx)
    if (
      (language === "bash" && ch === "#" && (i === 0 || line[i - 1] === " ")) ||
      ((language === "typescript" || language === "tsx") && ch === "/" && line[i + 1] === "/")
    ) {
      tokens.push({ content: line.slice(i), offset: i, tone: "muted" });
      break;
    }

    // Quoted strings
    if (ch === '"' || ch === "'" || ch === "`") {
      const quote = ch;
      let j = i + 1;
      while (j < n) {
        if (line[j] === "\\" && j + 1 < n) {
          j += 2;
          continue;
        }
        if (line[j] === quote) {
          j += 1;
          break;
        }
        j += 1;
      }
      const rest = line.slice(j).trimStart();
      const isJsonKey = language === "json" && rest.startsWith(":");
      tokens.push({
        content: line.slice(i, j),
        offset: i,
        tone: isJsonKey ? "accent" : "default",
      });
      i = j;
      continue;
    }

    // Words & keywords
    if (isWordStart(ch)) {
      let j = i + 1;
      while (j < n && isWordCont(line[j])) j += 1;
      const word = line.slice(i, j);
      tokens.push({
        content: word,
        offset: i,
        tone: KEYWORDS.has(word) ? "accent" : "default",
      });
      i = j;
      continue;
    }

    // Plain span
    let j = i + 1;
    while (
      j < n &&
      !isWordStart(line[j]) &&
      line[j] !== '"' &&
      line[j] !== "'" &&
      line[j] !== "`" &&
      line[j] !== "#" &&
      !(line[j] === "/" && line[j + 1] === "/")
    ) {
      j += 1;
    }
    tokens.push({ content: line.slice(i, j), offset: i, tone: "muted" });
    i = j;
  }

  return tokens;
}

export function AgentCodeLine({
  code,
  tokens,
  className,
}: AgentCodeLineProps) {
  return (
    <span className={className}>
      {tokens
        ? tokens.map((token) => (
            <span
              key={`${token.offset}-${token.content}`}
              style={token.tone && token.tone !== "default" ? { color: TONE_STYLE[token.tone] } : undefined}
            >
              {token.content}
            </span>
          ))
        : code}
    </span>
  );
}

export function AgentCode({
  code,
  language = "bash",
  className,
}: AgentCodeProps) {
  let offset = 0;
  const lines = code.split("\n").map((content) => {
    const line = { content, offset, tokens: tokenizeLine(content, language) };
    offset += content.length + 1;
    return line;
  });

  return (
    <pre
      className={cn(
        "m-0 overflow-x-auto whitespace-pre font-mono text-xs leading-5 text-foreground/85",
        className,
      )}
    >
      <code>
        {lines.map((line, index) => (
          <Fragment key={line.offset}>
            <AgentCodeLine code={line.content} tokens={line.tokens} />
            {index < lines.length - 1 ? "\n" : null}
          </Fragment>
        ))}
      </code>
    </pre>
  );
}
