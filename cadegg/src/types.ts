export type PanelState = "collapsed" | "expanded";
export type View = "chat" | "settings";
export type Provider = "claude" | "gemini" | "glm";

export interface ToolCall {
  id: string;
  name: string;
  args: Record<string, unknown>;
}

export interface SessionObject {
  handle: string;
  kind: string;
  label: string;
  source?: "generated" | "selection" | string;
}

export type ObjectUpdate =
  | { action: "upsert"; object: SessionObject }
  | { action: "remove"; handle: string }
  | { action: "remove_last" };

export type Message =
  | { role: "user"; content: string }
  | { role: "plan"; text: string | null; tool_calls: ToolCall[] }
  | { role: "assistant"; text: string | null; tool_calls: ToolCall[] }
  | {
      role: "tool";
      id: string;
      name: string;
      ok: boolean;
      content: string;
      confirmation_required: boolean;
      object_updates: ObjectUpdate[];
      pending_call?: ToolCall;
      confirmed?: boolean;
    };

export type HistoryMessage =
  | { role: "user"; content: string }
  | { role: "assistant"; text: string | null; tool_calls: ToolCall[] }
  | { role: "tool"; id: string; name: string; ok: boolean; content: string };

export type AgentEvent =
  | { kind: "assistant_delta"; delta: string }
  | { kind: "assistant"; text: string | null; tool_calls: ToolCall[] }
  | {
      kind: "tool_result";
      result: {
        id: string;
        name: string;
        ok: boolean;
        content: string;
        confirmation_required: boolean;
        object_updates: ObjectUpdate[];
      };
    }
  | { kind: "done"; text: string }
  | { kind: "error"; message: string };

export interface SettingsView {
  provider: Provider;
  model: string;
  base_url: string;
  gemini_model: string;
  gemini_base_url: string;
  glm_model: string;
  glm_base_url: string;
  anthropic_api_key_set: boolean;
  anthropic_api_key_preview: string;
  gemini_api_key_set: boolean;
  gemini_api_key_preview: string;
  glm_api_key_set: boolean;
  glm_api_key_preview: string;
}
