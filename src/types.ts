export type PanelState = "collapsed" | "expanded";
export type View = "chat" | "settings" | "help";
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
  | { kind: "assistant_trace"; delta: string }
  | { kind: "assistant_delta"; delta: string }
  | { kind: "usage"; usage: ProviderTokenUsage }
  | { kind: "model_route"; route: ModelRouteTelemetry }
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
  duration_ms?: number;
  token_telemetry?: TokenTelemetry;
}

export interface TokenTelemetry {
  started_at: number;
  total_duration_ms?: number;
  first_response_ms?: number;
  avg_chunk_gap_ms?: number;
  max_chunk_gap_ms?: number;
  chunk_count: number;
  input_tokens?: number;
  input_tokens_estimated?: boolean;
  output_tokens?: number;
  output_tokens_estimated?: boolean;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  reasoning_tokens?: number;
  provider_calls?: number;
  provider_models?: string[];
  output_tokens_per_second?: number;
  throughput_estimated?: boolean;
  estimated_context_tokens?: number;
}

export interface ProviderTokenUsage {
  provider: string;
  model: string;
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  reasoning_tokens?: number;
}

export interface ModelRouteAttempt {
  provider: string;
  model: string;
  status: "planned" | "attempting" | "selected" | "fallback" | "skipped" | "failed";
  reason?: string;
}

export interface ModelRouteTelemetry {
  selected_provider: string;
  selected_model: string;
  final_provider?: string;
  final_model?: string;
  fallback_count: number;
  attempts: ModelRouteAttempt[];
  note?: string;
}

export interface MemoryFileInfo {
  name: string;
  size_bytes: number;
  updated_at_ms: number;
}

export interface MemoryBundleInfo {
  dir: string;
  files: MemoryFileInfo[];
  global_memory: string;
  global_memory_exists: boolean;
}

export interface BenchmarkCandidate {
  provider: string;
  provider_label: string;
  model: string;
  skip_reason?: string;
}

export interface BenchmarkCaseResult {
  id: string;
  label: string;
  score: number;
  note: string;
}

export interface BenchmarkModelResult {
  provider: string;
  provider_label: string;
  model: string;
  requests: number;
  succeeded: number;
  avg_duration_ms: number;
  avg_output_tokens?: number;
  score: number;
  rating: number;
  cases: BenchmarkCaseResult[];
  errors: string[];
}

export interface BenchmarkSummary {
  started_at_ms: number;
  finished_at_ms?: number;
  cancelled: boolean;
  candidates_total: number;
  models_tested: number;
  max_requests: number;
  models: BenchmarkModelResult[];
  results_json_path: string;
  results_md_path: string;
}

export interface BenchmarkEvent {
  kind: string;
  current: number;
  total: number;
  provider?: string;
  model?: string;
  message: string;
}

export interface SettingsView {
  provider: Provider;
  work_mode: WorkMode;
  auto_failover: boolean;
  memory_carry_token_budget: number;
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
