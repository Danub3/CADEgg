export type PanelState = "collapsed" | "expanded";
export type View = "chat" | "settings";
export type Provider = "glm" | "deepseek" | "qwen" | "kimi";
export type WorkMode = "competition_mode" | "safety_demo_mode";

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

export interface ValidationCheck {
  id: string;
  label: string;
  passed: boolean;
}

export interface ElevatorValidation {
  ok: boolean;
  issues: string[];
  checks: ValidationCheck[];
  material_table: {
    guard_door: string;
    toe_board_height: number;
    warning_sign: boolean;
    material_table_included: boolean;
  };
}

export interface DemoLogEntry {
  time: string;
  user_input: string;
  tool_calls: string[];
  params: Record<string, unknown>;
  validation?: ElevatorValidation;
  summary: string;
}

export interface SettingsView {
  provider: Provider;
  work_mode: WorkMode;
  glm_model: string;
  glm_strong_model: string;
  glm_base_url: string;
  deepseek_model: string;
  deepseek_strong_model: string;
  deepseek_base_url: string;
  qwen_model: string;
  qwen_strong_model: string;
  qwen_base_url: string;
  kimi_model: string;
  kimi_strong_model: string;
  kimi_base_url: string;
  glm_api_key_set: boolean;
  glm_api_key_preview: string;
  deepseek_api_key_set: boolean;
  deepseek_api_key_preview: string;
  qwen_api_key_set: boolean;
  qwen_api_key_preview: string;
  kimi_api_key_set: boolean;
  kimi_api_key_preview: string;
}
