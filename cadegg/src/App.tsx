import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

import type {
  AgentEvent,
  DemoLogEntry,
  ElevatorValidation,
  Message,
  ObjectUpdate,
  PanelState,
  Provider,
  SessionObject,
  SettingsView,
  ToolCall,
  View,
} from "./types";
import { CLAUDE_MODELS, DEFAULT_VIEW, GEMINI_MODELS, GLM_MODELS } from "./constants";
import {
  applyObjectUpdates,
  cloneSessionObjects,
  getObjectReferenceHints,
  mergeSessionObjects,
  sourceDisplayLabel,
} from "./sessionObjects";
import { buildHistoryPayload, shouldAutoSyncObjectTable } from "./messages";
import { compactToolArgs, planSummary } from "./formatting";

export default function App() {
  const [panelState, setPanelState] = useState<PanelState>("collapsed");
  const [view, setView] = useState<View>("chat");

  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [assistantDraft, setAssistantDraft] = useState("");
  const [sessionObjects, setSessionObjects] = useState<SessionObject[]>([]);
  const [sending, setSending] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const [settings, setSettings] = useState<SettingsView>(DEFAULT_VIEW);
  const [savedHint, setSavedHint] = useState(false);
  const [claudeKeyDraft, setClaudeKeyDraft] = useState<string | null>(null);
  const [geminiKeyDraft, setGeminiKeyDraft] = useState<string | null>(null);
  const [glmKeyDraft, setGlmKeyDraft] = useState<string | null>(null);

  const [testStatus, setTestStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [syncingObjects, setSyncingObjects] = useState(false);
  const [importingSelection, setImportingSelection] = useState(false);

  const [demoLog, setDemoLog] = useState<DemoLogEntry[]>([]);
  const [lastValidation, setLastValidation] = useState<ElevatorValidation | null>(null);

  const collapseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const assistantDraftRef = useRef("");
  const sessionObjectsRef = useRef<SessionObject[]>([]);
  const pendingUndoSnapshotRef = useRef<SessionObject[] | null>(null);
  const undoSnapshotsRef = useRef<SessionObject[][]>([]);
  const runTouchedObjectTableRef = useRef(false);
  const pendingPostRunSyncRef = useRef(false);
  const pendingToolCallsRef = useRef<Record<string, ToolCall>>({});
  const lastUserInputRef = useRef("");
  const pendingLogRef = useRef<{
    toolCalls: string[];
    params: Record<string, unknown>;
    validation: ElevatorValidation | null;
    summary: string;
  } | null>(null);

  function tryParseValidation(content: string): ElevatorValidation | null {
    const trimmed = content.trim();
    if (!trimmed.startsWith("{")) return null;
    try {
      const parsed = JSON.parse(trimmed);
      if (
        parsed &&
        typeof parsed.ok === "boolean" &&
        Array.isArray(parsed.checks) &&
        typeof parsed.material_table === "object"
      ) {
        return parsed as ElevatorValidation;
      }
    } catch {
      // ignore
    }
    return null;
  }

  function updateSessionObjects(
    updater: SessionObject[] | ((prev: SessionObject[]) => SessionObject[])
  ) {
    setSessionObjects((prev) => {
      const next = typeof updater === "function" ? updater(prev) : updater;
      sessionObjectsRef.current = next;
      return next;
    });
  }

  function commitUndoSnapshotIfNeeded() {
    if (runTouchedObjectTableRef.current && pendingUndoSnapshotRef.current) {
      undoSnapshotsRef.current.push(cloneSessionObjects(pendingUndoSnapshotRef.current));
    }
    pendingUndoSnapshotRef.current = null;
    runTouchedObjectTableRef.current = false;
  }

  async function refreshSettings() {
    try {
      const s = await invoke<SettingsView>("get_settings");
      setSettings({ ...DEFAULT_VIEW, ...s });
      setClaudeKeyDraft(null);
      setGeminiKeyDraft(null);
      setGlmKeyDraft(null);
    } catch (e) {
      console.error("load settings:", e);
    }
  }

  useEffect(() => {
    refreshSettings();
  }, []);

  // Subscribe to agent streaming events from the Rust backend.
  // listen() resolves async — under React StrictMode dev mode the first cleanup
  // runs before the promise resolves, so we must remember "cancelled" and
  // unlisten as soon as the handle arrives.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    listen<AgentEvent>("agent:event", (ev) => {
      const e = ev.payload;
      if (e.kind === "assistant_delta") {
        assistantDraftRef.current += e.delta;
        setAssistantDraft(assistantDraftRef.current);
      } else if (e.kind === "assistant") {
        for (const call of e.tool_calls) {
          pendingToolCallsRef.current[call.id] = call;
          if (pendingLogRef.current) {
            pendingLogRef.current.toolCalls.push(call.name);
            Object.assign(pendingLogRef.current.params, call.args);
          }
        }
        if (e.tool_calls.length > 0 || (e.text && assistantDraftRef.current && e.text === assistantDraftRef.current)) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        setMessages((prev) => [
          ...prev,
          e.tool_calls.length > 0
            ? { role: "plan", text: e.text, tool_calls: e.tool_calls }
            : { role: "assistant", text: e.text, tool_calls: e.tool_calls },
        ]);
      } else if (e.kind === "tool_result") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        const pendingCall = e.result.confirmation_required
          ? pendingToolCallsRef.current[e.result.id]
          : undefined;
        if (e.result.name === "validate_elevator_shaft_protection") {
          const validation = tryParseValidation(e.result.content);
          if (validation) {
            setLastValidation(validation);
            if (pendingLogRef.current) {
              pendingLogRef.current.validation = validation;
            }
          }
        }
        if (e.result.name === "draw_elevator_shaft_protection") {
          if (pendingLogRef.current) {
            pendingLogRef.current.summary = e.result.ok
              ? e.result.content
              : `绘图失败：${e.result.content}`;
          }
        }
        setMessages((prev) => [
          ...prev,
          { role: "tool", ...e.result, pending_call: pendingCall },
        ]);
        if (e.result.ok && shouldAutoSyncObjectTable(e.result.name)) {
          runTouchedObjectTableRef.current = true;
          pendingPostRunSyncRef.current = true;
        }
        if (e.result.object_updates.length > 0) {
          updateSessionObjects((prev) => applyObjectUpdates(prev, e.result.object_updates));
        }
      } else if (e.kind === "done") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        commitUndoSnapshotIfNeeded();
        const pending = pendingLogRef.current;
        pendingLogRef.current = null;
        if (pending && pending.toolCalls.length > 0) {
          const entry: DemoLogEntry = {
            time: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
            user_input: lastUserInputRef.current,
            tool_calls: pending.toolCalls,
            params: pending.params,
            validation: pending.validation ?? undefined,
            summary: pending.summary || "完成",
          };
          setDemoLog((prev) => [entry, ...prev].slice(0, 30));
        }
        const shouldPostRunSync = pendingPostRunSyncRef.current;
        pendingPostRunSyncRef.current = false;
        if (shouldPostRunSync) {
          void (async () => {
            await syncSessionObjects(false);
            setSending(false);
          })();
        } else {
          setSending(false);
        }
      } else if (e.kind === "error") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        commitUndoSnapshotIfNeeded();
        pendingPostRunSyncRef.current = false;
        setErrorMsg(e.message);
        setSending(false);
        if (e.message.includes("API Key")) setView("settings");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages, sending]);

  function handleMouseEnter() {
    if (collapseTimer.current) clearTimeout(collapseTimer.current);
    if (panelState === "collapsed") setPanelState("expanded");
  }

  function handleMouseLeave() {
    if (collapseTimer.current) clearTimeout(collapseTimer.current);
    collapseTimer.current = setTimeout(() => {
      if (panelRef.current?.contains(document.activeElement)) return;
      if (view === "chat" && !sending) setPanelState("collapsed");
    }, 800);
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const text = input.trim();
    if (!text || sending) return;

    const newHistory: Message[] = [...messages, { role: "user", content: text }];
    setMessages(newHistory);
    setInput("");
    assistantDraftRef.current = "";
    setAssistantDraft("");
    setSending(true);
    setErrorMsg(null);

    try {
      let syncedObjects = sessionObjectsRef.current;
      if (syncedObjects.length > 0) {
        const latest = await syncSessionObjects(false);
        if (latest === null) {
          setSending(false);
          return;
        }
        syncedObjects = latest;
      }

    pendingUndoSnapshotRef.current = cloneSessionObjects(syncedObjects);
    runTouchedObjectTableRef.current = false;
    pendingPostRunSyncRef.current = false;
    lastUserInputRef.current = text;
    pendingLogRef.current = {
      toolCalls: [],
      params: {},
      validation: null,
      summary: "",
    };

      // run_agent emits agent:event for each step; resolve only signals the loop ended.
      // We send the prior history (without the new user msg) — backend appends user_input itself.
      await invoke("run_agent", {
        userInput: text,
        history: buildHistoryPayload(messages),
        sessionObjects: syncedObjects,
      });
    } catch (e) {
      pendingUndoSnapshotRef.current = null;
      runTouchedObjectTableRef.current = false;
      pendingPostRunSyncRef.current = false;
      setErrorMsg(String(e));
      setSending(false);
    }
  }

  async function saveSettings() {
    try {
      await invoke("save_settings", {
        update: {
          provider: settings.provider,
          work_mode: settings.work_mode,
          model: settings.model,
          base_url: settings.base_url,
          gemini_model: settings.gemini_model,
          gemini_base_url: settings.gemini_base_url,
          glm_model: settings.glm_model,
          glm_base_url: settings.glm_base_url,
          anthropic_api_key: claudeKeyDraft,
          gemini_api_key: geminiKeyDraft,
          glm_api_key: glmKeyDraft,
        },
      });
      setSavedHint(true);
      setTimeout(() => setSavedHint(false), 1500);
      await refreshSettings();
    } catch (e) {
      setErrorMsg(String(e));
    }
  }

  async function runCadAction(cmd: "test_cad_connection" | "draw_test_line") {
    setTestStatus({ ok: true, msg: "执行中..." });
    try {
      const msg = await invoke<string>(cmd);
      setTestStatus({ ok: true, msg });
    } catch (e) {
      setTestStatus({ ok: false, msg: String(e) });
    }
  }

  async function syncSessionObjects(showStatus: boolean) {
    const current = sessionObjectsRef.current;
    if (current.length === 0) {
      if (showStatus) {
        setTestStatus({ ok: true, msg: "当前没有可同步的会话对象" });
      }
      return current;
    }

    setSyncingObjects(true);
    if (showStatus) {
      setTestStatus({ ok: true, msg: "对象表同步中..." });
    }
    setErrorMsg(null);

    try {
      const synced = await invoke<SessionObject[]>("sync_session_objects", {
        sessionObjects: current,
      });
      updateSessionObjects(synced);
      if (showStatus) {
        const removed = current.length - synced.length;
        setTestStatus({
          ok: true,
          msg:
            removed > 0
              ? `对象表已同步，移除了 ${removed} 个已不存在对象`
              : `对象表已同步，${synced.length} 个对象仍有效`,
        });
      }
      return synced;
    } catch (e) {
      const msg = String(e);
      if (showStatus) {
        setTestStatus({ ok: false, msg });
      }
      setErrorMsg(msg);
      return null;
    } finally {
      setSyncingObjects(false);
    }
  }

  async function handleImportSelectedObjects() {
    if (sending || undoing || syncingObjects || importingSelection) return;
    setImportingSelection(true);
    setTestStatus({ ok: true, msg: "导入选中对象中..." });
    setErrorMsg(null);

    try {
      const before = sessionObjectsRef.current;
      const imported = await invoke<SessionObject[]>("import_selected_objects");
      const added = imported.filter(
        (object) => !before.some((existing) => existing.handle === object.handle)
      ).length;
      const refreshed = imported.length - added;

      updateSessionObjects((prev) => mergeSessionObjects(prev, imported));

      setTestStatus({
        ok: true,
        msg:
          refreshed > 0
            ? `已导入 ${imported.length} 个选中对象，其中 ${added} 个新增、${refreshed} 个已更新`
            : `已导入 ${imported.length} 个选中对象`,
      });
    } catch (e) {
      const msg = String(e);
      setTestStatus({ ok: false, msg });
      setErrorMsg(msg);
    } finally {
      setImportingSelection(false);
    }
  }

  async function handleUndoLastGeneration() {
    if (sending || undoing || syncingObjects || importingSelection) return;
    setUndoing(true);
    setTestStatus({ ok: true, msg: "撤回中..." });
    setErrorMsg(null);
    try {
      const msg = await invoke<string>("undo_last_generation");
      setTestStatus({ ok: true, msg });
      setMessages((prev) => [...prev, { role: "assistant", text: msg, tool_calls: [] }]);
      const snapshot = undoSnapshotsRef.current.pop();
      if (snapshot) {
        updateSessionObjects(snapshot);
      }
      await syncSessionObjects(false);
    } catch (e) {
      const msg = String(e);
      setTestStatus({ ok: false, msg });
      setErrorMsg(msg);
    } finally {
      setUndoing(false);
    }
  }

  async function handleConfirmToolCall(messageIndex: number, call: ToolCall) {
    if (sending || undoing || syncingObjects || importingSelection) return;
    setSending(true);
    setErrorMsg(null);
    try {
      const result = await invoke<{
        id: string;
        name: string;
        ok: boolean;
        content: string;
        confirmation_required: boolean;
        object_updates: ObjectUpdate[];
      }>("confirm_tool_call", { call });

      setMessages((prev) =>
        prev.map((message, index) =>
          index === messageIndex && message.role === "tool"
            ? { ...message, confirmed: true }
            : message
        ).concat({ role: "tool", ...result })
      );

      if (result.ok && shouldAutoSyncObjectTable(result.name)) {
        runTouchedObjectTableRef.current = true;
      }
      if (result.object_updates.length > 0) {
        updateSessionObjects((prev) => applyObjectUpdates(prev, result.object_updates));
      }
      if (result.ok && shouldAutoSyncObjectTable(result.name)) {
        await syncSessionObjects(false);
      }
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setSending(false);
    }
  }

  const isExpanded = panelState !== "collapsed";
  const accent =
    settings.provider === "gemini"
      ? "from-sky-500 to-violet-500"
      : settings.provider === "glm"
      ? "from-emerald-400 to-teal-500"
      : "from-orange-400 to-rose-500";
  const accentSolid =
    settings.provider === "gemini"
      ? "bg-violet-600 hover:bg-violet-500"
      : settings.provider === "glm"
      ? "bg-teal-600 hover:bg-teal-500"
      : "bg-rose-500 hover:bg-rose-400";
  const providerLabel =
    settings.provider === "gemini" ? "Gemini" : settings.provider === "glm" ? "GLM" : "Claude";
  const objectReferenceHints = getObjectReferenceHints(sessionObjects);

  return (
    <div className="absolute right-0 top-0 h-full flex items-center justify-end pointer-events-none">
      {/* Collapsed tab */}
      <div
        onMouseEnter={handleMouseEnter}
        className={`absolute right-0 flex items-center justify-center w-9 h-20 rounded-l-2xl bg-gradient-to-b ${accent} text-white text-sm font-semibold cursor-pointer shadow-xl ring-1 ring-white/40 transition-all duration-200 pointer-events-auto ${
          isExpanded ? "opacity-0 translate-x-2 pointer-events-none" : "opacity-100 translate-x-0"
        }`}
      >
        <span className="rotate-180 [writing-mode:vertical-rl] tracking-widest text-[11px]">
          CAD·Egg
        </span>
      </div>

      {/* Expanded panel */}
      <div
        ref={panelRef}
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
        className={`relative flex flex-col w-[340px] h-[94vh] mr-3 rounded-3xl bg-white/85 backdrop-blur-2xl text-slate-800 shadow-[0_20px_60px_-15px_rgba(15,23,42,0.35)] ring-1 ring-slate-200/70 overflow-hidden transition-all duration-300 origin-right pointer-events-auto ${
          isExpanded ? "scale-100 opacity-100 translate-x-0" : "scale-95 opacity-0 translate-x-6 pointer-events-none"
        }`}
      >
        {/* Header */}
        <div className="flex items-center gap-2.5 px-4 py-3.5 border-b border-slate-200/80">
          <div className={`w-7 h-7 rounded-xl bg-gradient-to-br ${accent} shadow-sm flex items-center justify-center text-white text-[11px] font-bold`}>
            ✱
          </div>
          <div className="flex-1 leading-tight">
            <div className="text-sm font-semibold text-slate-800">CADEgg</div>
            <div className="text-[10px] text-slate-500">{providerLabel} · AutoCAD Agent</div>
          </div>
          <button
            className="w-7 h-7 rounded-lg text-slate-500 hover:text-slate-800 hover:bg-slate-100 flex items-center justify-center text-base transition-colors"
            title={view === "chat" ? "设置" : "返回对话"}
            onClick={() => setView(view === "chat" ? "settings" : "chat")}
          >
            {view === "chat" ? "⚙" : "←"}
          </button>
        </div>

        {/* Chat view */}
        {view === "chat" && (
          <>
            <div ref={scrollRef} className="flex-1 px-4 py-3 flex flex-col gap-3 overflow-y-auto">
              {/* CAD connection row */}
              <div className="rounded-2xl bg-slate-50/80 border border-slate-200/70 p-3 flex flex-col gap-2">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] text-slate-500 uppercase tracking-wider font-medium">CAD 连接</span>
                  <span className="text-[10px] text-slate-400">桥接优先 · COM 回退</span>
                </div>
                <div className="flex gap-1.5">
                  <button
                    className="flex-1 py-1.5 rounded-lg bg-white border border-slate-200 hover:border-slate-300 hover:bg-slate-50 text-xs text-slate-700 font-medium shadow-sm transition-colors"
                    onClick={() => runCadAction("test_cad_connection")}
                  >
                    连接
                  </button>
                  <button
                    className="flex-1 py-1.5 rounded-lg bg-white border border-slate-200 hover:border-slate-300 hover:bg-slate-50 text-xs text-slate-700 font-medium shadow-sm transition-colors"
                    onClick={() => runCadAction("draw_test_line")}
                  >
                    画线
                  </button>
                </div>
                <button
                  className="w-full py-1.5 rounded-lg bg-slate-900 hover:bg-slate-800 disabled:bg-slate-200 disabled:text-slate-400 text-xs text-white font-medium shadow-sm transition-colors"
                  onClick={handleUndoLastGeneration}
                  disabled={sending || undoing || syncingObjects}
                >
                  {undoing ? "撤回中..." : "撤回上一次生成"}
                </button>
                {testStatus && (
                  <div
                    className={`px-2.5 py-1.5 rounded-lg text-[11px] font-mono break-all ${
                      testStatus.ok
                        ? "bg-emerald-50 border border-emerald-200 text-emerald-700"
                        : "bg-rose-50 border border-rose-200 text-rose-700"
                    }`}
                  >
                    {testStatus.msg}
                  </div>
                )}
              </div>

              <div className="rounded-2xl bg-amber-50/70 border border-amber-200/80 p-3 flex flex-col gap-2">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] text-amber-700 uppercase tracking-wider font-medium">
                    会话对象
                  </span>
                  <span className="text-[10px] text-amber-600">
                    {sessionObjects.length > 0
                      ? `${sessionObjects.length} 个可引用对象`
                      : "尚无可引用对象"}
                  </span>
                </div>
                <div className="flex gap-1.5">
                  <button
                    className="flex-1 py-1.5 rounded-lg bg-white/85 border border-amber-200 text-[10px] text-amber-900 font-medium hover:bg-white disabled:bg-white/60 disabled:text-amber-400 transition-colors"
                    onClick={handleImportSelectedObjects}
                    disabled={sending || undoing || syncingObjects || importingSelection}
                  >
                    {importingSelection ? "导入中" : "导入选中"}
                  </button>
                  <button
                    className="flex-1 py-1.5 rounded-lg bg-white/85 border border-amber-200 text-[10px] text-amber-800 font-medium hover:bg-white disabled:bg-white/60 disabled:text-amber-400 transition-colors"
                    onClick={() => void syncSessionObjects(true)}
                    disabled={sending || undoing || syncingObjects || importingSelection || sessionObjects.length === 0}
                  >
                    {syncingObjects ? "同步中" : "同步"}
                  </button>
                </div>
                {sessionObjects.length > 0 ? (
                  <div className="flex flex-col gap-1.5 max-h-36 overflow-y-auto pr-0.5">
                    {sessionObjects.map((object) => (
                      <div
                        key={object.handle}
                        className="rounded-xl bg-white/80 border border-amber-200/70 px-2.5 py-2 text-[11px] text-slate-700"
                      >
                        <div className="flex items-center gap-2">
                          <span className="px-1.5 py-0.5 rounded-md bg-amber-100 text-amber-800 font-mono">
                            {object.handle}
                          </span>
                          <span className="text-[10px] font-semibold text-amber-700">
                            {object.kind}
                          </span>
                          <span className="text-[10px] text-slate-500">
                            {sourceDisplayLabel(object.source)}
                          </span>
                        </div>
                        <div className="mt-1 leading-relaxed break-words">{object.label}</div>
                        <div className="mt-1 flex flex-wrap gap-1">
                          {(objectReferenceHints.get(object.handle) ?? []).slice(0, 3).map((hint) => (
                            <span
                              key={hint}
                              className="px-1.5 py-0.5 rounded-md bg-slate-100 text-[10px] text-slate-600"
                            >
                              {hint}
                            </span>
                          ))}
                        </div>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="rounded-xl bg-white/70 border border-dashed border-amber-200 px-3 py-2 text-[11px] text-amber-800 leading-relaxed">
                    先在 AutoCAD 里选中对象，再点“导入选中”，这些对象就会进入当前会话对象表，后续对话可继续引用。
                  </div>
                )}
              </div>

              {/* Validation panel */}
              {lastValidation && (
                <div
                  className={`rounded-2xl border p-3 flex flex-col gap-2 ${
                    lastValidation.ok
                      ? "bg-emerald-50/70 border-emerald-200/80"
                      : "bg-rose-50/70 border-rose-200/80"
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <span className="text-[10px] uppercase tracking-wider font-medium text-slate-600">
                      安全防护校核
                    </span>
                    <span
                      className={`text-[11px] font-semibold ${
                        lastValidation.ok ? "text-emerald-700" : "text-rose-700"
                      }`}
                    >
                      {lastValidation.ok ? "✓ 通过" : "✗ 未通过"}
                    </span>
                  </div>
                  <div className="flex flex-col gap-1">
                    {lastValidation.checks.map((check) => (
                      <div
                        key={check.id}
                        className="flex items-start gap-1.5 text-[11px] leading-snug"
                      >
                        <span
                          className={`shrink-0 font-semibold ${
                            check.passed ? "text-emerald-600" : "text-rose-600"
                          }`}
                        >
                          {check.passed ? "✓" : "✗"}
                        </span>
                        <span className="text-slate-700">{check.label}</span>
                      </div>
                    ))}
                  </div>
                  {lastValidation.issues.length > 0 && (
                    <div className="rounded-lg bg-rose-100/70 px-2 py-1.5 text-[11px] text-rose-800">
                      风险项：{lastValidation.issues.join("；")}
                    </div>
                  )}
                  <div className="rounded-lg bg-white/70 border border-slate-200/70 px-2 py-1.5 text-[11px] text-slate-700">
                    材料表：立杆 {lastValidation.material_table.posts} 根 ·{" "}
                    {lastValidation.material_table.rails === "top_and_mid_rails"
                      ? "上横杆+中横杆"
                      : lastValidation.material_table.rails}
                    {" · "}踢脚板 {lastValidation.material_table.toe_board_height}mm · 警示牌{" "}
                    {lastValidation.material_table.warning_sign ? "已配" : "未配"}
                  </div>
                </div>
              )}

              {/* Demo log */}
              {demoLog.length > 0 && (
                <details className="rounded-2xl bg-slate-50/80 border border-slate-200/70 p-3">
                  <summary className="cursor-pointer text-[10px] uppercase tracking-wider font-medium text-slate-500 list-none">
                    演示履历 · {demoLog.length} 条
                  </summary>
                  <div className="mt-2 flex flex-col gap-2">
                    {demoLog.map((entry, idx) => (
                      <div
                        key={idx}
                        className="rounded-xl bg-white/80 border border-slate-200/70 px-2.5 py-2 text-[11px] text-slate-700"
                      >
                        <div className="flex items-center justify-between">
                          <span className="text-slate-500">{entry.time}</span>
                          <span
                            className={`font-semibold ${
                              entry.validation
                                ? entry.validation.ok
                                  ? "text-emerald-600"
                                  : "text-rose-600"
                                : "text-slate-500"
                            }`}
                          >
                            {entry.validation
                              ? entry.validation.ok
                                ? "校核通过"
                                : "校核未通过"
                              : "已执行"}
                          </span>
                        </div>
                        <div className="mt-1 break-words text-slate-600">{entry.user_input}</div>
                        <div className="mt-1 flex flex-wrap gap-1">
                          {entry.tool_calls.map((name) => (
                            <span
                              key={name}
                              className="px-1.5 py-0.5 rounded-md bg-violet-50 text-violet-700 font-mono text-[10px]"
                            >
                              {name}
                            </span>
                          ))}
                        </div>
                      </div>
                    ))}
                  </div>
                </details>
              )}

              {/* Messages */}
              {messages.length === 0 && (
                <div className="flex-1 flex flex-col items-center justify-center text-center py-6 gap-2">
                  <div className={`w-12 h-12 rounded-2xl bg-gradient-to-br ${accent} opacity-90 shadow-lg flex items-center justify-center text-white text-xl`}>
                    ✦
                  </div>
                  <p className="text-slate-600 text-sm font-medium">开始一段 CAD 对话</p>
                  <p className="text-slate-400 text-xs leading-relaxed">
                    在 AutoCAD 中选中对象，<br />或直接告诉我你想画什么
                  </p>
                </div>
              )}
              {messages.map((m, i) => renderMessage(m, i, accent, handleConfirmToolCall))}
              {assistantDraft && (
                <details className="self-start max-w-[88%] rounded-2xl bg-slate-50 border border-slate-200/80 shadow-sm px-3.5 py-2 text-slate-700">
                  <summary className="cursor-pointer text-xs font-medium text-slate-500 list-none">
                    思考中
                  </summary>
                  <div className="mt-2 text-sm whitespace-pre-wrap break-words leading-relaxed">
                    {assistantDraft}
                  </div>
                </details>
              )}
              {sending && (
                <div className="self-start flex items-center gap-1.5 px-3 py-2 rounded-2xl bg-white border border-slate-200/80 shadow-sm">
                  <span className="w-1.5 h-1.5 rounded-full bg-slate-400 animate-bounce [animation-delay:-0.3s]" />
                  <span className="w-1.5 h-1.5 rounded-full bg-slate-400 animate-bounce [animation-delay:-0.15s]" />
                  <span className="w-1.5 h-1.5 rounded-full bg-slate-400 animate-bounce" />
                </div>
              )}
              {errorMsg && (
                <div className="px-3 py-2 rounded-xl bg-rose-50 border border-rose-200 text-rose-700 text-xs font-mono break-all">
                  {errorMsg}
                </div>
              )}
            </div>

            <form
              onSubmit={handleSubmit}
              className="px-3 py-3 border-t border-slate-200/80 bg-white/60 flex gap-2 items-end"
            >
              <textarea
                rows={1}
                className="flex-1 resize-none bg-white text-slate-800 text-sm placeholder-slate-400 rounded-2xl px-3.5 py-2 outline-none border border-slate-200 focus:border-slate-400 focus:ring-2 focus:ring-slate-200 transition-all max-h-28"
                placeholder={sending ? "等待回复..." : "画一条 7000mm 的直线..."}
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    handleSubmit(e as unknown as React.FormEvent);
                  }
                }}
                disabled={sending}
              />
              <button
                type="submit"
                disabled={!input.trim() || sending}
                className={`w-9 h-9 rounded-full ${accentSolid} disabled:bg-slate-200 disabled:text-slate-400 text-white text-base shadow-md flex items-center justify-center transition-all`}
              >
                ↑
              </button>
            </form>
          </>
        )}

        {/* Settings drawer overlay */}
        <div
          className={`absolute inset-0 bg-white flex flex-col transition-transform duration-300 ease-out ${
            view === "settings" ? "translate-x-0" : "translate-x-full"
          }`}
        >
          <div className="flex items-center gap-2.5 px-4 py-3.5 border-b border-slate-200/80">
            <button
              className="w-7 h-7 rounded-lg text-slate-500 hover:text-slate-800 hover:bg-slate-100 flex items-center justify-center transition-colors"
              onClick={() => setView("chat")}
            >
              ←
            </button>
            <span className="text-sm font-semibold text-slate-800">设置</span>
          </div>

          <div className="flex-1 px-4 py-4 flex flex-col gap-5 overflow-y-auto">
            {/* Provider switch */}
            <div className="flex flex-col gap-2">
              <label className="text-xs text-slate-500 font-medium">模型提供方</label>
              <div className="grid grid-cols-2 gap-1.5 p-1 bg-slate-100 rounded-xl">
                {(["gemini", "glm"] as Provider[]).map((p) => {
                  const active = settings.provider === p;
                  const label = p === "gemini" ? "Gemini" : "GLM";
                  return (
                    <button
                      key={p}
                      onClick={() => setSettings({ ...settings, provider: p })}
                      className={`py-1.5 rounded-lg text-xs font-medium transition-all ${
                        active
                          ? "bg-white text-slate-800 shadow-sm"
                          : "text-slate-500 hover:text-slate-700"
                      }`}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>
              {settings.provider === "glm" && (
                <p className="text-[10px] text-emerald-700 leading-snug">
                  💡 智谱 GLM-4-Flash / 4.5-Flash 有免费额度，国内直连不用代理。
                </p>
              )}
            </div>

            <div className="flex flex-col gap-2">
              <label className="text-xs text-slate-500 font-medium">工作模式</label>
              <div className="grid grid-cols-2 gap-1.5 p-1 bg-slate-100 rounded-xl">
                {(["competition_mode", "safety_demo_mode"] as const).map((mode) => {
                  const active = settings.work_mode === mode;
                  const label = mode === "competition_mode" ? "比赛模式" : "安全防护 demo";
                  return (
                    <button
                      key={mode}
                      onClick={() =>
                        setSettings({
                          ...settings,
                          work_mode: mode,
                          provider: settings.provider === "claude" ? "glm" : settings.provider,
                        })
                      }
                      className={`py-1.5 rounded-lg text-xs font-medium transition-all ${
                        active
                          ? "bg-white text-slate-800 shadow-sm"
                          : "text-slate-500 hover:text-slate-700"
                      }`}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>
              <p className="text-[10px] text-slate-400 leading-snug">
                比赛模式隐藏 Claude 和 run_lisp；安全防护 demo 只开放电梯井口临边防护闭环工具。
              </p>
            </div>

            {settings.provider === "claude" ? (
              <>
                <KeyField
                  label="Anthropic API Key"
                  isSet={settings.anthropic_api_key_set}
                  preview={settings.anthropic_api_key_preview}
                  draft={claudeKeyDraft}
                  onDraftChange={setClaudeKeyDraft}
                  placeholder="sk-ant-..."
                />

                <Field label="API Base URL" hint="官方留空即可；中转站填到 /v1/messages 之前的部分">
                  <input
                    type="text"
                    className={inputCls}
                    placeholder="https://api.anthropic.com"
                    value={settings.base_url}
                    onChange={(e) => setSettings({ ...settings, base_url: e.target.value })}
                  />
                </Field>

                <Field label="模型" hint="中转站若不允许指定模型，请留空">
                  <input
                    type="text"
                    list="claude-models"
                    className={inputCls}
                    placeholder="claude-opus-4-7（中转站可留空）"
                    value={settings.model}
                    onChange={(e) => setSettings({ ...settings, model: e.target.value })}
                  />
                  <datalist id="claude-models">
                    {CLAUDE_MODELS.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.label}
                      </option>
                    ))}
                  </datalist>
                </Field>
              </>
            ) : settings.provider === "gemini" ? (
              <>
                <KeyField
                  label="Gemini API Key"
                  isSet={settings.gemini_api_key_set}
                  preview={settings.gemini_api_key_preview}
                  draft={geminiKeyDraft}
                  onDraftChange={setGeminiKeyDraft}
                  placeholder="AIza..."
                />

                <Field
                  label="API Base URL"
                  hint="官方地址在中国大陆被墙。需走代理或填入 Gemini 中转站地址"
                >
                  <input
                    type="text"
                    className={inputCls}
                    placeholder="https://generativelanguage.googleapis.com"
                    value={settings.gemini_base_url}
                    onChange={(e) =>
                      setSettings({ ...settings, gemini_base_url: e.target.value })
                    }
                  />
                </Field>

                <Field label="模型" hint="2.5-pro 需付费；2.5-flash / 2.0-flash 免费层可用">
                  <input
                    type="text"
                    list="gemini-models"
                    className={inputCls}
                    placeholder="gemini-2.0-flash"
                    value={settings.gemini_model}
                    onChange={(e) =>
                      setSettings({ ...settings, gemini_model: e.target.value })
                    }
                  />
                  <datalist id="gemini-models">
                    {GEMINI_MODELS.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.label}
                      </option>
                    ))}
                  </datalist>
                </Field>
              </>
            ) : (
              <>
                <KeyField
                  label="GLM API Key"
                  isSet={settings.glm_api_key_set}
                  preview={settings.glm_api_key_preview}
                  draft={glmKeyDraft}
                  onDraftChange={setGlmKeyDraft}
                  placeholder="xxxxxxxx.xxxx（在 bigmodel.cn 控制台获取）"
                />

                <Field
                  label="API Base URL"
                  hint="智谱官方端点；中转/反代请改这里"
                >
                  <input
                    type="text"
                    className={inputCls}
                    placeholder="https://open.bigmodel.cn/api/paas/v4"
                    value={settings.glm_base_url}
                    onChange={(e) =>
                      setSettings({ ...settings, glm_base_url: e.target.value })
                    }
                  />
                </Field>

                <Field label="模型" hint="glm-4-flash / 4.5-flash 免费；glm-4.5 / 4-plus 付费">
                  <input
                    type="text"
                    list="glm-models"
                    className={inputCls}
                    placeholder="glm-4-flash"
                    value={settings.glm_model}
                    onChange={(e) =>
                      setSettings({ ...settings, glm_model: e.target.value })
                    }
                  />
                  <datalist id="glm-models">
                    {GLM_MODELS.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.label}
                      </option>
                    ))}
                  </datalist>
                </Field>
              </>
            )}

            <p className="text-[10px] text-slate-400 leading-snug">
              🔒 API Key 仅保存在本机 AppData/settings.json。界面不会明文回显已保存的 key，发送请求时由 Rust 后端直接加到 HTTP 头，永不暴露给前端 JS。
            </p>
          </div>

          <div className="px-3 py-3 border-t border-slate-200/80 flex gap-2 items-center bg-white">
            <span className="flex-1 text-xs text-emerald-600 font-medium transition-opacity">
              {savedHint ? "✓ 已保存" : ""}
            </span>
            <button
              onClick={saveSettings}
              className={`px-5 py-2 rounded-xl ${accentSolid} text-white text-sm font-medium shadow-md transition-colors`}
            >
              保存
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function renderMessage(
  m: Message,
  i: number,
  accent: string,
  onConfirmToolCall: (messageIndex: number, call: ToolCall) => void
) {
  if (m.role === "user") {
    return (
      <div
        key={i}
        className={`max-w-[88%] px-3.5 py-2 rounded-2xl text-sm whitespace-pre-wrap break-words leading-relaxed self-end bg-gradient-to-br ${accent} text-white shadow-md`}
      >
        {m.content}
      </div>
    );
  }
  if (m.role === "plan") {
    return (
      <details
        key={i}
        className="self-start max-w-[88%] rounded-2xl bg-slate-50 border border-slate-200/80 shadow-sm px-3 py-2"
      >
        <summary className="cursor-pointer text-xs font-medium text-slate-500 list-none">
          执行计划 · {m.tool_calls.length} 步 · {planSummary(m.tool_calls)}
        </summary>
        <div className="mt-2 flex flex-col gap-1.5">
          {m.text && (
            <div className="text-sm whitespace-pre-wrap break-words leading-relaxed text-slate-700">
              {m.text}
            </div>
          )}
          {m.tool_calls.map((tc) => (
            <div
              key={tc.id}
              className="px-3 py-2 rounded-xl bg-violet-50 border border-violet-200 text-[11px] font-mono text-violet-900"
            >
              <div className="font-semibold mb-0.5">{tc.name}</div>
              <div className="opacity-75 break-words">{compactToolArgs(tc.args)}</div>
            </div>
          ))}
        </div>
      </details>
    );
  }
  if (m.role === "assistant") {
    return (
      <div key={i} className="self-start max-w-[88%] flex flex-col gap-1.5">
        {m.text && (
          <div className="px-3.5 py-2 rounded-2xl text-sm whitespace-pre-wrap break-words leading-relaxed bg-white text-slate-700 border border-slate-200/80 shadow-sm">
            {m.text}
          </div>
        )}
        {m.tool_calls.map((tc) => (
          <div
            key={tc.id}
            className="px-3 py-2 rounded-xl bg-violet-50 border border-violet-200 text-[11px] font-mono text-violet-900 break-all"
          >
            <div className="font-semibold mb-0.5">🔧 {tc.name}</div>
            <div className="opacity-75">{JSON.stringify(tc.args)}</div>
          </div>
        ))}
      </div>
    );
  }
  // tool result
  return (
    <div
      key={i}
      className={`self-start max-w-[88%] px-3 py-2 rounded-xl text-[11px] font-mono break-all ${
        m.confirmation_required
          ? "bg-amber-50 border border-amber-200 text-amber-900"
          : m.ok
          ? "bg-emerald-50 border border-emerald-200 text-emerald-800"
          : "bg-rose-50 border border-rose-200 text-rose-800"
      }`}
    >
      <span className="font-semibold">
        {m.confirmation_required ? "!" : m.ok ? "✓" : "✗"} {m.name}:{" "}
      </span>
      {m.content}
      {m.confirmation_required && m.pending_call && !m.confirmed && (
        <button
          type="button"
          onClick={() => onConfirmToolCall(i, m.pending_call!)}
          className="mt-2 w-full py-1.5 rounded-lg bg-amber-600 hover:bg-amber-500 text-white text-[11px] font-semibold transition-colors"
        >
          确认执行
        </button>
      )}
      {m.confirmation_required && m.confirmed && (
        <div className="mt-2 text-[10px] text-amber-700">已确认，结果见下方。</div>
      )}
    </div>
  );
}

const inputCls =
  "w-full bg-slate-50 text-slate-800 text-sm rounded-xl px-3 py-2 outline-none border border-slate-200 focus:border-slate-400 focus:bg-white focus:ring-2 focus:ring-slate-200 font-mono transition-all";

function KeyField({
  label,
  isSet,
  preview,
  draft,
  onDraftChange,
  placeholder,
}: {
  label: string;
  isSet: boolean;
  preview: string;
  draft: string | null;
  onDraftChange: (v: string | null) => void;
  placeholder: string;
}) {
  const editing = draft !== null;
  const hint = isSet
    ? editing
      ? "正在修改 —— 保存后覆盖原 key"
      : "已保存。点右侧按钮修改，原 key 不会被读回前端。"
    : "本地保存到 AppData，不上传任何服务器";

  return (
    <Field label={label} hint={hint}>
      {editing ? (
        <div className="relative">
          <input
            type="password"
            className={inputCls + " pr-20"}
            placeholder={placeholder}
            value={draft ?? ""}
            onChange={(e) => onDraftChange(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            autoFocus
          />
          <button
            type="button"
            onClick={() => onDraftChange(null)}
            className="absolute right-1.5 top-1/2 -translate-y-1/2 px-2 py-1 rounded-md text-[10px] text-slate-500 hover:text-slate-800 hover:bg-slate-100 font-medium transition-colors"
          >
            取消
          </button>
        </div>
      ) : (
        <div className="flex gap-1.5 items-center">
          <div
            className={`${inputCls} flex-1 font-mono flex items-center ${
              isSet ? "text-slate-600" : "text-slate-400"
            }`}
          >
            {isSet ? preview : "（未设置）"}
          </div>
          <button
            type="button"
            onClick={() => onDraftChange("")}
            className="px-3 py-2 rounded-xl bg-slate-100 hover:bg-slate-200 text-slate-700 text-xs font-medium transition-colors whitespace-nowrap"
          >
            {isSet ? "修改" : "设置"}
          </button>
        </div>
      )}
    </Field>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-xs text-slate-500 font-medium">{label}</label>
      {children}
      {hint && <span className="text-[10px] text-slate-400 leading-snug">{hint}</span>}
    </div>
  );
}
