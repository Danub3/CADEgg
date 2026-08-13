use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::tools::{self, SessionObject, ToolCall, ToolResult};

const MAX_TURNS: usize = 8;
const SESSION_OBJECT_CONTEXT_LIMIT: usize = 8;

const SYSTEM_PROMPT: &str = "你是 CADEgg，一名 AutoCAD 助理。\n\
- 用户讲中文，你也用中文回答。\n\
- 当用户的请求需要操作 CAD 时，调用提供的工具，不要凭空编造对象信息。\n\
- 坐标和尺寸的默认单位是毫米，除非用户明确说米。\n\
- 优先使用结构化工具（draw_line/draw_circle/move 等）；只有当结构化工具明显不足以表达需求时才用 run_lisp。\n\
- 工具库是分层的：能用高层语义几何工具时，不要退回到底层原子工具去自己算；需要读对象信息时优先用查询工具。\n\
- 如果一个请求可以拆成多步且这些步骤都已确定，尽量在同一轮里一次性返回完整的 tool_calls，不要只做第一步。\n\
- 当前会话如果额外提供了对象表，就把它当作可引用对象清单；对象不在表里时，不要假设它还存在。\n\
- 如果用户要求引用当前在 AutoCAD 里手工选中的对象，且这些对象还不在对象表里，先用 import_selection 把它们导入再继续操作。\n\
- 如果消息里出现“系统引用解析”，把其中给出的 handle 视为对用户指代的显式解析结果，优先按它执行。\n\
- 如果工具结果里出现 handle，后续继续操作同一对象时优先使用按 handle 的工具（如 move_handle / erase_handle），不要退回 last。\n\
- 在调用工具前，先用一句话说明你打算做什么；调用结束后，用一句简短的确认结尾。\n\
- 不要重复调用同一个工具去验证结果；工具的返回字符串就是事实。";

// ---------------- Wire format (frontend ↔ backend) ----------------

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum MessageView {
    User {
        content: String,
    },
    Assistant {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        name: String,
        ok: bool,
        content: String,
    },
}

// ---------------- Provider trait via enum (no async_trait dep) ----------------

#[derive(Clone, Debug)]
pub enum StepOutput {
    /// Final natural-language answer with no further tool calls.
    Text(String),
    /// Optional accompanying text + tool invocations to execute.
    ToolCalls {
        text: Option<String>,
        calls: Vec<ToolCall>,
    },
}

#[derive(Clone)]
struct ExecutedBatch {
    signature: String,
    summary: String,
}

enum Provider {
    Gemini(GeminiProvider),
    Glm(GlmProvider),
    Claude,
}

impl Provider {
    async fn step(&self, history: &[MessageView], app: &AppHandle) -> Result<StepOutput, String> {
        match self {
            Provider::Gemini(g) => g.step(history, app).await,
            Provider::Glm(g) => g.step(history, app).await,
            Provider::Claude => {
                Err("Claude tool-use 暂未实现，请在设置里切换到 Gemini 或 GLM".to_string())
            }
        }
    }
}

struct GeminiProvider {
    api_key: String,
    model: String,
    base_url: String,
    selected_tools: Vec<String>,
}

impl GeminiProvider {
    async fn step(&self, history: &[MessageView], app: &AppHandle) -> Result<StepOutput, String> {
        if self.api_key.trim().is_empty() {
            return Err("尚未配置 Gemini API Key".to_string());
        }
        if self.model.trim().is_empty() {
            return Err("Gemini 必须指定模型 ID".to_string());
        }

        let contents = history_to_gemini_contents(history);
        let body = json!({
            "systemInstruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
            "contents": contents,
            "tools": tools::gemini_function_declarations_for(&self.selected_tools),
        });

        let base = self.base_url.trim_end_matches('/');
        let url = format!(
            "{base}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.model
        );

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("网络请求失败: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("读响应失败: {e}"))?;
            return Err(format!("Gemini API {status}: {text}"));
        }
        let mut text_buf = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut saw_event = false;
        let stream_result = read_sse_events(resp, |data| {
            saw_event = true;
            let parsed: Value =
                serde_json::from_str(data).map_err(|e| format!("解析流式响应失败: {e}"))?;
            let parts = parsed["candidates"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|c| c["content"]["parts"].as_array())
                .cloned()
                .unwrap_or_default();
            for (i, p) in parts.iter().enumerate() {
                if let Some(t) = p["text"].as_str() {
                    if !t.is_empty() {
                        text_buf.push_str(t);
                        let _ = app.emit("agent:event", AgentEvent::AssistantDelta { delta: t });
                    }
                } else if let Some(fc) = p.get("functionCall") {
                    let name = fc["name"].as_str().unwrap_or("").to_string();
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    let call_id = format!("g-{i}");
                    if let Some(existing) = calls.iter_mut().find(|existing| existing.id == call_id)
                    {
                        existing.name = name;
                        existing.args = args;
                    } else {
                        calls.push(ToolCall {
                            id: call_id,
                            name,
                            args,
                        });
                    }
                }
            }
            Ok(())
        })
        .await;

        if let Err(error) = stream_result {
            if !saw_event {
                let fallback_url = format!("{base}/v1beta/models/{}:generateContent", self.model);
                let fallback_resp = client
                    .post(&fallback_url)
                    .header("x-goog-api-key", &self.api_key)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("网络请求失败: {e}"))?;
                let fallback_status = fallback_resp.status();
                let fallback_text = fallback_resp
                    .text()
                    .await
                    .map_err(|e| format!("读响应失败: {e}"))?;
                if !fallback_status.is_success() {
                    return Err(format!("Gemini API {fallback_status}: {fallback_text}"));
                }
                let fallback_parsed: Value = serde_json::from_str(&fallback_text)
                    .map_err(|e| format!("解析响应失败: {e}"))?;
                return parse_gemini_step_output(&fallback_parsed, &fallback_text);
            }
            return Err(error);
        }

        if !calls.is_empty() {
            let final_text = if text_buf.trim().is_empty() {
                None
            } else {
                Some(text_buf)
            };
            Ok(StepOutput::ToolCalls {
                text: final_text,
                calls,
            })
        } else if !text_buf.trim().is_empty() {
            Ok(StepOutput::Text(text_buf))
        } else {
            Err("Gemini 返回为空".to_string())
        }
    }
}

fn history_to_gemini_contents(history: &[MessageView]) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in history {
        match msg {
            MessageView::User { content } => out.push(json!({
                "role": "user",
                "parts": [{ "text": content }],
            })),
            MessageView::Assistant { text, tool_calls } => {
                let mut parts: Vec<Value> = Vec::new();
                if let Some(t) = text.as_ref().filter(|s| !s.trim().is_empty()) {
                    parts.push(json!({ "text": t }));
                }
                for c in tool_calls {
                    parts.push(json!({
                        "functionCall": { "name": c.name, "args": c.args }
                    }));
                }
                if parts.is_empty() {
                    continue;
                }
                out.push(json!({ "role": "model", "parts": parts }));
            }
            MessageView::Tool {
                name, ok, content, ..
            } => {
                let response_payload = if *ok {
                    json!({ "content": content })
                } else {
                    json!({ "error": content })
                };
                out.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": response_payload,
                        }
                    }],
                }));
            }
        }
    }
    out
}

// ---------------- Zhipu GLM (OpenAI-compatible) ----------------

struct GlmProvider {
    api_key: String,
    model: String,
    base_url: String,
    selected_tools: Vec<String>,
}

impl GlmProvider {
    async fn step(&self, history: &[MessageView], app: &AppHandle) -> Result<StepOutput, String> {
        if self.api_key.trim().is_empty() {
            return Err("尚未配置 GLM API Key".to_string());
        }
        if self.model.trim().is_empty() {
            return Err("GLM 必须指定模型 ID".to_string());
        }

        let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
        messages.extend(history_to_openai_messages(history));

        let body = json!({
            "model": self.model,
            "messages": messages.clone(),
            "tools": tools::openai_tools_for(&self.selected_tools),
            "tool_choice": "auto",
            "stream": true,
        });

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/chat/completions");

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("网络请求失败: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("读响应失败: {e}"))?;
            return Err(format!("GLM API {status}: {text}"));
        }
        let mut text_buf = String::new();
        let mut calls: Vec<PendingOpenAiToolCall> = Vec::new();
        let mut saw_event = false;
        let stream_result = read_sse_events(resp, |data| {
            if data == "[DONE]" {
                saw_event = true;
                return Ok(());
            }
            saw_event = true;
            let parsed: Value =
                serde_json::from_str(data).map_err(|e| format!("解析流式响应失败: {e}"))?;
            let choice = parsed["choices"]
                .as_array()
                .and_then(|a| a.first())
                .ok_or_else(|| format!("流式响应缺少 choices[0]: {data}"))?;
            let delta = &choice["delta"];
            if let Some(content) = delta["content"].as_str() {
                if !content.is_empty() {
                    text_buf.push_str(content);
                    let _ = app.emit("agent:event", AgentEvent::AssistantDelta { delta: content });
                }
            }
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for (pos, tc) in tool_calls.iter().enumerate() {
                    let index = tc["index"].as_u64().map(|v| v as usize).unwrap_or(pos);
                    while calls.len() <= index {
                        calls.push(PendingOpenAiToolCall::default());
                    }
                    let pending = &mut calls[index];
                    if let Some(id) = tc["id"].as_str() {
                        pending.id = id.to_string();
                    }
                    if let Some(name) = tc["function"]["name"].as_str() {
                        pending.name = name.to_string();
                    }
                    if let Some(arguments) = tc["function"]["arguments"].as_str() {
                        pending.arguments.push_str(arguments);
                    }
                }
            }
            Ok(())
        })
        .await;

        if let Err(error) = stream_result {
            if !saw_event {
                let fallback_body = json!({
                    "model": self.model,
                    "messages": messages,
                    "tools": tools::openai_tools_for(&self.selected_tools),
                    "tool_choice": "auto",
                });
                let fallback_resp = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("content-type", "application/json")
                    .json(&fallback_body)
                    .send()
                    .await
                    .map_err(|e| format!("网络请求失败: {e}"))?;
                let fallback_status = fallback_resp.status();
                let fallback_text = fallback_resp
                    .text()
                    .await
                    .map_err(|e| format!("读响应失败: {e}"))?;
                if !fallback_status.is_success() {
                    return Err(format!("GLM API {fallback_status}: {fallback_text}"));
                }
                let fallback_parsed: Value = serde_json::from_str(&fallback_text)
                    .map_err(|e| format!("解析响应失败: {e}"))?;
                return parse_glm_step_output(&fallback_parsed, &fallback_text);
            }
            return Err(error);
        }

        let calls: Vec<ToolCall> = calls
            .into_iter()
            .enumerate()
            .map(|(i, pending)| ToolCall {
                id: if pending.id.is_empty() {
                    format!("glm-{i}")
                } else {
                    pending.id
                },
                name: pending.name,
                args: serde_json::from_str(&pending.arguments).unwrap_or(json!({})),
            })
            .filter(|call| !call.name.is_empty())
            .collect();

        if !calls.is_empty() {
            let final_text = if text_buf.trim().is_empty() {
                None
            } else {
                Some(text_buf)
            };
            Ok(StepOutput::ToolCalls {
                text: final_text,
                calls,
            })
        } else if !text_buf.trim().is_empty() {
            Ok(StepOutput::Text(text_buf))
        } else {
            Err("GLM 返回为空".to_string())
        }
    }
}

fn history_to_openai_messages(history: &[MessageView]) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in history {
        match msg {
            MessageView::User { content } => out.push(json!({
                "role": "user",
                "content": content,
            })),
            MessageView::Assistant { text, tool_calls } => {
                let mut m = json!({
                    "role": "assistant",
                    "content": text.as_deref().unwrap_or(""),
                });
                if !tool_calls.is_empty() {
                    let tcs: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": c.args.to_string(),
                                }
                            })
                        })
                        .collect();
                    m["tool_calls"] = Value::Array(tcs);
                    // OpenAI convention: content may be null when tool_calls present
                    if text.is_none() {
                        m["content"] = Value::Null;
                    }
                }
                out.push(m);
            }
            MessageView::Tool { id, content, .. } => out.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": content,
            })),
        }
    }
    out
}

// ---------------- Streamed step events to the frontend ----------------

#[derive(Default)]
struct PendingOpenAiToolCall {
    id: String,
    name: String,
    arguments: String,
}

fn parse_gemini_step_output(parsed: &Value, raw_text: &str) -> Result<StepOutput, String> {
    let parts = parsed["candidates"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c["content"]["parts"].as_array())
        .cloned()
        .unwrap_or_default();

    let mut text_buf = String::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if let Some(t) = p["text"].as_str() {
            text_buf.push_str(t);
        } else if let Some(fc) = p.get("functionCall") {
            let name = fc["name"].as_str().unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or(json!({}));
            calls.push(ToolCall {
                id: format!("g-{i}"),
                name,
                args,
            });
        }
    }

    if !calls.is_empty() {
        let final_text = if text_buf.trim().is_empty() {
            None
        } else {
            Some(text_buf)
        };
        Ok(StepOutput::ToolCalls {
            text: final_text,
            calls,
        })
    } else if !text_buf.trim().is_empty() {
        Ok(StepOutput::Text(text_buf))
    } else {
        Err(format!("Gemini 返回为空: {raw_text}"))
    }
}

fn parse_glm_step_output(parsed: &Value, raw_text: &str) -> Result<StepOutput, String> {
    let msg = parsed["choices"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .ok_or_else(|| format!("响应缺少 choices[0].message: {raw_text}"))?;

    let content = msg["content"].as_str().unwrap_or("").trim().to_string();
    let mut calls: Vec<ToolCall> = Vec::new();
    if let Some(tcs) = msg["tool_calls"].as_array() {
        for tc in tcs {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            calls.push(ToolCall { id, name, args });
        }
    }

    if !calls.is_empty() {
        let final_text = if content.is_empty() {
            None
        } else {
            Some(content)
        };
        Ok(StepOutput::ToolCalls {
            text: final_text,
            calls,
        })
    } else if !content.is_empty() {
        Ok(StepOutput::Text(content))
    } else {
        Err(format!("GLM 返回为空: {raw_text}"))
    }
}

async fn read_sse_events<F>(mut resp: reqwest::Response, mut on_data: F) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let mut buffer = String::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取流失败: {e}"))? {
        buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        while let Some(idx) = buffer.find("\n\n") {
            let raw_event = buffer[..idx].to_string();
            buffer.drain(..idx + 2);
            if let Some(data) = extract_sse_data(&raw_event) {
                on_data(&data)?;
            }
        }
    }

    if !buffer.trim().is_empty() {
        if let Some(data) = extract_sse_data(&buffer) {
            on_data(&data)?;
        }
    }
    Ok(())
}

fn extract_sse_data(raw_event: &str) -> Option<String> {
    let mut data_lines: Vec<String> = Vec::new();
    for line in raw_event.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AgentEvent<'a> {
    AssistantDelta {
        delta: &'a str,
    },
    Assistant {
        text: Option<&'a str>,
        tool_calls: &'a [ToolCall],
    },
    ToolResult {
        result: &'a ToolResult,
    },
    Done {
        text: &'a str,
    },
    Error {
        message: &'a str,
    },
}

fn redact(s: &str, settings: &crate::settings::Settings) -> String {
    let mut out = s.to_string();
    for key in [
        settings.anthropic_api_key.trim(),
        settings.gemini_api_key.trim(),
        settings.glm_api_key.trim(),
    ] {
        if key.len() >= 8 {
            out = out.replace(key, "***REDACTED***");
        }
    }
    if let Some(idx) = out.find("key=") {
        let tail = &out[idx + 4..];
        let end = tail
            .find(|c: char| c == '&' || c == ' ' || c == '"' || c == ')')
            .unwrap_or(tail.len());
        let span = &tail[..end];
        if !span.is_empty() {
            out = out.replace(&format!("key={span}"), "key=***REDACTED***");
        }
    }
    out
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(v).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{}:{}", key, canonical_json(&map[key])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn tool_call_batch_signature(calls: &[ToolCall]) -> String {
    calls
        .iter()
        .map(|call| format!("{}({})", call.name, canonical_json(&call.args)))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn summarize_tool_results(results: &[ToolResult]) -> String {
    if results.is_empty() {
        return "已完成。".to_string();
    }

    let ok_parts: Vec<&str> = results
        .iter()
        .filter(|r| r.ok)
        .map(|r| r.content.as_str())
        .collect();
    let err_parts: Vec<&str> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| r.content.as_str())
        .collect();

    if err_parts.is_empty() {
        if ok_parts.len() == 1 {
            format!("已完成：{}", ok_parts[0])
        } else {
            format!("已完成：{}", ok_parts.join("；"))
        }
    } else if ok_parts.is_empty() {
        format!("操作失败：{}", err_parts.join("；"))
    } else {
        format!(
            "已完成部分操作：{}；失败：{}",
            ok_parts.join("；"),
            err_parts.join("；")
        )
    }
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(
        name,
        "draw_line"
            | "draw_circle"
            | "draw_regular_polygon"
            | "draw_equilateral_triangle_about_circle"
            | "draw_rectangle_by_center"
            | "draw_elevator_shaft_protection"
            | "draw_text"
            | "move"
            | "move_handle"
            | "rotate_handle"
            | "copy_handle"
            | "mirror_handle"
            | "offset_handle"
            | "trim_by_handle"
            | "extend_by_handle"
            | "erase_last"
            | "erase_handle"
            | "zoom_extents"
            | "run_lisp"
    )
}

fn kind_reference_terms(kind: &str) -> Vec<String> {
    match kind {
        "LINE" => vec!["条线".to_string(), "条直线".to_string()],
        "CIRCLE" => vec!["个圆".to_string()],
        "ARC" => vec!["段圆弧".to_string()],
        "LWPOLYLINE" => vec!["条多段线".to_string()],
        other => vec![format!("个{}对象", other)],
    }
}

fn source_display_name(source: Option<&str>) -> &'static str {
    match source {
        Some("generated") => "创建",
        Some("selection") => "导入",
        _ => "纳入",
    }
}

fn ordinal_cn(n: usize) -> Option<&'static str> {
    match n {
        1 => Some("一"),
        2 => Some("二"),
        3 => Some("三"),
        4 => Some("四"),
        5 => Some("五"),
        6 => Some("六"),
        7 => Some("七"),
        8 => Some("八"),
        9 => Some("九"),
        10 => Some("十"),
        11 => Some("十一"),
        12 => Some("十二"),
        _ => None,
    }
}

fn push_unique_string(into: &mut Vec<String>, value: String) {
    if !into.iter().any(|existing| existing == &value) {
        into.push(value);
    }
}

fn numbered_aliases(prefix: &str, n: usize, suffix: &str) -> Vec<String> {
    let mut aliases = vec![format!("{prefix}{n}{suffix}")];
    if let Some(cn) = ordinal_cn(n) {
        aliases.push(format!("{prefix}{cn}{suffix}"));
    }
    aliases
}

fn build_object_aliases(
    object_index: usize,
    kind: &str,
    kind_index: usize,
    source: Option<&str>,
    source_index: usize,
) -> Vec<String> {
    let mut aliases = Vec::new();

    for alias in numbered_aliases("第", object_index, "个对象") {
        push_unique_string(&mut aliases, alias);
    }
    if object_index == 1 {
        push_unique_string(&mut aliases, "最新对象".to_string());
    }

    for term in kind_reference_terms(kind) {
        for alias in numbered_aliases("第", kind_index, &term) {
            push_unique_string(&mut aliases, alias);
        }
        if kind_index == 1 {
            push_unique_string(&mut aliases, format!("最新{term}"));
            push_unique_string(&mut aliases, format!("上一{term}"));
        }
    }

    let source_name = source_display_name(source);
    for alias in numbered_aliases("第", source_index, &format!("个{source_name}对象")) {
        push_unique_string(&mut aliases, alias);
    }
    if source_index == 1 {
        push_unique_string(&mut aliases, format!("最新{source_name}的对象"));
        push_unique_string(&mut aliases, format!("刚{source_name}的对象"));
    }

    aliases
}

fn build_context_aliases(
    object_index: usize,
    kind: &str,
    kind_index: usize,
    source: Option<&str>,
    source_index: usize,
) -> Vec<String> {
    let mut aliases = Vec::new();

    if object_index == 1 {
        push_unique_string(&mut aliases, "最新对象".to_string());
    }
    if let Some(alias) = numbered_aliases("第", object_index, "个对象")
        .into_iter()
        .next()
    {
        push_unique_string(&mut aliases, alias);
    }

    if let Some(term) = kind_reference_terms(kind).into_iter().next() {
        if kind_index == 1 {
            push_unique_string(&mut aliases, format!("最新{term}"));
        }
        if let Some(alias) = numbered_aliases("第", kind_index, &term).into_iter().next() {
            push_unique_string(&mut aliases, alias);
        }
    }

    let source_name = source_display_name(source);
    if source_index == 1 {
        push_unique_string(&mut aliases, format!("最新{source_name}的对象"));
    }
    if let Some(alias) = numbered_aliases("第", source_index, &format!("个{source_name}对象"))
        .into_iter()
        .next()
    {
        push_unique_string(&mut aliases, alias);
    }

    aliases
}

#[derive(Clone)]
struct ResolvedObjectReference {
    start: usize,
    end: usize,
    phrase: String,
    handle: String,
    kind: String,
    label: String,
}

fn resolve_user_object_references(
    user_input: &str,
    session_objects: &[SessionObject],
) -> Vec<ResolvedObjectReference> {
    let mut kind_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let mut source_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let mut candidates: Vec<ResolvedObjectReference> = Vec::new();

    for (idx, obj) in session_objects.iter().enumerate() {
        let kind_index = kind_counts
            .entry(obj.kind.as_str())
            .and_modify(|count| *count += 1)
            .or_insert(1);
        let source_key = obj.source.as_deref().unwrap_or("session");
        let source_index = source_counts
            .entry(source_key)
            .and_modify(|count| *count += 1)
            .or_insert(1);

        for alias in build_object_aliases(
            idx + 1,
            &obj.kind,
            *kind_index,
            obj.source.as_deref(),
            *source_index,
        ) {
            if let Some(start) = user_input.find(&alias) {
                candidates.push(ResolvedObjectReference {
                    start,
                    end: start + alias.len(),
                    phrase: alias,
                    handle: obj.handle.clone(),
                    kind: obj.kind.clone(),
                    label: obj.label.clone(),
                });
            }
        }
    }

    candidates.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| b.phrase.len().cmp(&a.phrase.len()))
    });

    let mut resolved: Vec<ResolvedObjectReference> = Vec::new();
    for candidate in candidates {
        let overlaps = resolved
            .iter()
            .any(|existing| candidate.start < existing.end && existing.start < candidate.end);
        let duplicate_phrase = resolved.iter().any(|existing| {
            existing.phrase == candidate.phrase && existing.handle == candidate.handle
        });
        if !overlaps && !duplicate_phrase {
            resolved.push(candidate);
        }
    }

    resolved
}

fn format_resolved_object_reference_context(
    user_input: &str,
    session_objects: &[SessionObject],
) -> Option<String> {
    let resolved = resolve_user_object_references(user_input, session_objects);
    if resolved.is_empty() {
        return None;
    }

    let lines: Vec<String> = resolved
        .iter()
        .map(|item| {
            format!(
                "- “{}” => handle={} type={} label={}",
                item.phrase, item.handle, item.kind, item.label
            )
        })
        .collect();

    Some(format!(
        "系统引用解析（已显式完成；后续优先按这些 handle 理解用户指代）：\n{}",
        lines.join("\n")
    ))
}

fn session_object_context(session_objects: &[SessionObject]) -> Option<String> {
    if session_objects.is_empty() {
        return None;
    }

    let mut kind_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let mut source_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let lines: Vec<String> = session_objects
        .iter()
        .take(SESSION_OBJECT_CONTEXT_LIMIT)
        .enumerate()
        .map(|(idx, obj)| {
            let kind_count = kind_counts
                .entry(obj.kind.as_str())
                .and_modify(|count| *count += 1)
                .or_insert(1);
            let source_key = obj.source.as_deref().unwrap_or("session");
            let source_count = source_counts
                .entry(source_key)
                .and_modify(|count| *count += 1)
                .or_insert(1);
            let aliases = build_context_aliases(
                idx + 1,
                &obj.kind,
                *kind_count,
                obj.source.as_deref(),
                *source_count,
            );

            format!(
                "{}. handle={} type={} refs={} label={}",
                idx + 1,
                obj.handle,
                obj.kind,
                aliases.join(" / "),
                obj.label
            )
        })
        .collect();

    let mut header =
        "当前会话可引用对象表（最近在前；generated=创建，selection=导入）。".to_string();
    if session_objects.len() > SESSION_OBJECT_CONTEXT_LIMIT {
        header.push_str(&format!(
            " 仅展开最近 {} 个，其余 {} 个未展开。",
            SESSION_OBJECT_CONTEXT_LIMIT,
            session_objects.len() - SESSION_OBJECT_CONTEXT_LIMIT
        ));
    }

    Some(format!("{header}\n{}", lines.join("\n")))
}

#[cfg(windows)]
fn begin_undo_group() -> Result<(), String> {
    crate::cad::cad_begin_undo_group()
}

#[cfg(not(windows))]
fn begin_undo_group() -> Result<(), String> {
    Err("UNDO GROUP 仅在 Windows 上可用".to_string())
}

#[cfg(windows)]
fn end_undo_group() -> Result<(), String> {
    crate::cad::cad_end_undo_group()
}

#[cfg(not(windows))]
fn end_undo_group() -> Result<(), String> {
    Err("UNDO GROUP 仅在 Windows 上可用".to_string())
}

// ---------------- Tauri command: run_agent ----------------

#[tauri::command]
pub async fn run_agent(
    app: AppHandle,
    user_input: String,
    history: Vec<MessageView>,
    session_objects: Vec<SessionObject>,
) -> Result<(), String> {
    let settings = crate::settings::load(&app)?;
    let user_text = user_input.trim().to_string();
    let tooling = tools::select_tooling_context(&user_text, &session_objects, settings.work_mode);
    let provider = match settings.provider.as_str() {
        "gemini" => Provider::Gemini(GeminiProvider {
            api_key: settings.gemini_api_key.clone(),
            model: settings.gemini_model.clone(),
            base_url: settings.gemini_base_url.clone(),
            selected_tools: tooling.tool_names.clone(),
        }),
        "glm" => Provider::Glm(GlmProvider {
            api_key: settings.glm_api_key.clone(),
            model: settings.glm_model.clone(),
            base_url: settings.glm_base_url.clone(),
            selected_tools: tooling.tool_names.clone(),
        }),
        _ => Provider::Claude,
    };

    let mut msgs = history;
    if let Some(context) = session_object_context(&session_objects) {
        msgs.push(MessageView::User { content: context });
    }
    if !tooling.guidance.trim().is_empty() {
        msgs.push(MessageView::User {
            content: format!("系统工具分层策略：\n{}", tooling.guidance),
        });
    }
    if tools::is_safety_request(&user_text) {
        if let Some(prompt) = tools::safety_clarification_prompt(&user_text) {
            msgs.push(MessageView::User {
                content: format!("系统提醒（安全防护缺参追问）：\n{}", prompt),
            });
        }
    }
    if !user_text.is_empty() {
        if let Some(reference_context) =
            format_resolved_object_reference_context(&user_text, &session_objects)
        {
            msgs.push(MessageView::User {
                content: reference_context,
            });
        }
        msgs.push(MessageView::User { content: user_text });
    }

    let mut undo_group_open = false;
    let mut last_executed_batch: Option<ExecutedBatch> = None;
    for _turn in 0..MAX_TURNS {
        let step = provider.step(&msgs, &app).await;
        let step = match step {
            Ok(s) => s,
            Err(e) => {
                if undo_group_open {
                    let _ = end_undo_group();
                }
                let msg = redact(&e, &settings);
                let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                return Err(msg);
            }
        };
        match step {
            StepOutput::Text(text) => {
                if undo_group_open {
                    if let Err(e) = end_undo_group() {
                        let msg = redact(&e, &settings);
                        let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                        return Err(msg);
                    }
                }
                let _ = app.emit(
                    "agent:event",
                    AgentEvent::Assistant {
                        text: Some(&text),
                        tool_calls: &[],
                    },
                );
                let _ = app.emit("agent:event", AgentEvent::Done { text: &text });
                return Ok(());
            }
            StepOutput::ToolCalls { text, calls } => {
                let signature = tool_call_batch_signature(&calls);
                if let Some(previous) = &last_executed_batch {
                    if previous.signature == signature {
                        if undo_group_open {
                            if let Err(e) = end_undo_group() {
                                let msg = redact(&e, &settings);
                                let _ =
                                    app.emit("agent:event", AgentEvent::Error { message: &msg });
                                return Err(msg);
                            }
                        }
                        let summary = format!(
                            "检测到模型重复生成同一批工具调用，已停止重复执行。{}",
                            previous.summary
                        );
                        let _ = app.emit(
                            "agent:event",
                            AgentEvent::Assistant {
                                text: Some(&summary),
                                tool_calls: &[],
                            },
                        );
                        let _ = app.emit("agent:event", AgentEvent::Done { text: &summary });
                        return Ok(());
                    }
                }
                if !calls.is_empty() && !undo_group_open {
                    if let Err(e) = begin_undo_group() {
                        let msg = redact(&e, &settings);
                        let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                        return Err(msg);
                    }
                    undo_group_open = true;
                }
                let _ = app.emit(
                    "agent:event",
                    AgentEvent::Assistant {
                        text: text.as_deref(),
                        tool_calls: &calls,
                    },
                );
                msgs.push(MessageView::Assistant {
                    text: None,
                    tool_calls: calls.clone(),
                });
                let mut batch_results: Vec<ToolResult> = Vec::new();
                for call in &calls {
                    let result = tools::dispatch_with_mode(call, settings.work_mode);
                    let _ = app.emit("agent:event", AgentEvent::ToolResult { result: &result });
                    msgs.push(MessageView::Tool {
                        id: result.id.clone(),
                        name: result.name.clone(),
                        ok: result.ok,
                        content: result.content.clone(),
                    });
                    batch_results.push(result);
                }
                let has_mutating_tools = calls.iter().any(|call| is_mutating_tool(&call.name));
                if has_mutating_tools {
                    if undo_group_open {
                        if let Err(e) = end_undo_group() {
                            let msg = redact(&e, &settings);
                            let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                            return Err(msg);
                        }
                    }
                    let done_text = "";
                    let _ = app.emit("agent:event", AgentEvent::Done { text: done_text });
                    return Ok(());
                }
                if !calls.is_empty() {
                    msgs.push(MessageView::User {
                        content: "系统提醒：上一步工具结果已返回。若工具已成功完成任务，请不要重复执行相同绘图工具，直接给出简短最终结果。".to_string(),
                    });
                }
                if !batch_results.is_empty() {
                    last_executed_batch = Some(ExecutedBatch {
                        signature,
                        summary: summarize_tool_results(&batch_results),
                    });
                }
            }
        }
    }

    if undo_group_open {
        let _ = end_undo_group();
    }
    let msg = format!("超出最大轮次 {MAX_TURNS}，循环已中止");
    let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
    Err(msg)
}

#[tauri::command]
pub fn confirm_tool_call(app: AppHandle, call: ToolCall) -> Result<ToolResult, String> {
    let settings = crate::settings::load(&app)?;
    let mut undo_group_open = false;
    if is_mutating_tool(&call.name) {
        begin_undo_group()?;
        undo_group_open = true;
    }

    let result = tools::dispatch_confirmed_with_mode(&call, settings.work_mode);

    if undo_group_open {
        if let Err(e) = end_undo_group() {
            return Err(e);
        }
    }

    Ok(result)
}
