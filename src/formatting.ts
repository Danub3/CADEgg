import type { ToolCall } from "./types";

function formatCompactValue(value: unknown): string {
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (typeof value === "string") return value.length > 18 ? `${value.slice(0, 18)}...` : value;
  if (Array.isArray(value)) return `[${value.length}]`;
  if (value && typeof value === "object") return "{...}";
  return "null";
}

export function compactToolArgs(args: Record<string, unknown>): string {
  const entries = Object.entries(args);
  if (entries.length === 0) return "无参数";

  const preview = entries
    .slice(0, 4)
    .map(([key, value]) => `${key}=${formatCompactValue(value)}`)
    .join(", ");

  return entries.length > 4 ? `${preview}, ...` : preview;
}

export function planSummary(toolCalls: ToolCall[]): string {
  if (toolCalls.length === 0) return "无工具";
  const names = toolCalls.slice(0, 3).map((call) => call.name).join(" -> ");
  return toolCalls.length > 3 ? `${names} ...` : names;
}
