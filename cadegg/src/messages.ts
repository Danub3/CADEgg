import type { HistoryMessage, Message } from "./types";

export function shouldAutoSyncObjectTable(toolName: string): boolean {
  return [
    "draw_line",
    "draw_circle",
    "draw_regular_polygon",
    "draw_equilateral_triangle_about_circle",
    "draw_rectangle_by_center",
    "draw_double_flight_stair",
    "draw_text",
    "move",
    "move_handle",
    "rotate_handle",
    "copy_handle",
    "mirror_handle",
    "offset_handle",
    "trim_by_handle",
    "extend_by_handle",
    "erase_last",
    "erase_handle",
    "run_lisp",
  ].includes(toolName);
}

export function buildHistoryPayload(messages: Message[]): HistoryMessage[] {
  return messages.map((message) => {
    switch (message.role) {
      case "user":
        return message;
      case "plan":
        return { role: "assistant", text: null, tool_calls: message.tool_calls };
      case "assistant":
        return message;
      case "tool":
        return {
          role: "tool",
          id: message.id,
          name: message.name,
          ok: message.ok,
          content: message.content,
        };
    }
  });
}
