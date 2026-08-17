import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type SetStateAction,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "./App.css";
import type {
  AgentEvent,
  DemoLogEntry,
  ElevatorValidation,
  Message,
  ObjectUpdate,
  Provider,
  SessionObject,
  SettingsView,
  ToolCall,
  View,
} from "./types";
import { DEFAULT_VIEW, MODEL_PROVIDERS, providerMeta } from "./constants";
import {
  applyObjectUpdates,
  cloneSessionObjects,
  getObjectReferenceHints,
  mergeSessionObjects,
  sourceDisplayLabel,
} from "./sessionObjects";
import { buildHistoryPayload, shouldAutoSyncObjectTable } from "./messages";
import { compactToolArgs, planSummary } from "./formatting";

type GlassBorderStyle = "pixel" | "glow";

interface GlassSettings {
  transparency: number;
  blur: number;
  border: GlassBorderStyle;
}

const GLASS_STORAGE_KEY = "cadegg.glassSettings.v1";
const APP_PREFS_STORAGE_KEY = "cadegg.appPreferences.v1";
const CHAT_SESSIONS_STORAGE_KEY = "cadegg.chatSessions.v1";
const DEFAULT_GLASS_SETTINGS: GlassSettings = {
  transparency: 45,
  blur: 70,
  border: "pixel",
};

interface AppPreferences {
  language: "zh-CN" | "en-US";
  fontSize: number;
  storageLocation: "appdata" | "project";
  notifications: boolean;
  autoSyncObjects: boolean;
  alwaysOnTop: boolean;
  reduceMotion: boolean;
  densePanels: boolean;
}

const DEFAULT_APP_PREFERENCES: AppPreferences = {
  language: "zh-CN",
  fontSize: 14,
  storageLocation: "appdata",
  notifications: true,
  autoSyncObjects: true,
  alwaysOnTop: false,
  reduceMotion: false,
  densePanels: false,
};

interface ChatSession {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  provider: Provider;
  model: string;
  messages: Message[];
  sessionObjects: SessionObject[];
  demoLog: DemoLogEntry[];
  lastValidation: ElevatorValidation | null;
  lastDrawParams: Record<string, unknown> | null;
}

interface StoredChatSessions {
  activeSessionId: string;
  sessions: ChatSession[];
}

function clamp(value: number, min: number, max: number) {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}

function makeId(prefix: string) {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function normalizeProvider(value: unknown): Provider {
  return MODEL_PROVIDERS.some((provider) => provider.id === value) ? (value as Provider) : "glm";
}

function normalizeSettingsView(value: Partial<SettingsView>): SettingsView {
  return {
    ...DEFAULT_VIEW,
    ...value,
    provider: normalizeProvider(value.provider),
  };
}

function currentModelFor(settings: SettingsView, provider: Provider = settings.provider) {
  const meta = providerMeta(provider);
  const value = settings[meta.strongModelField];
  return typeof value === "string" && value.trim()
    ? value
    : String(DEFAULT_VIEW[meta.strongModelField] ?? "");
}

function createChatSession(settings: SettingsView): ChatSession {
  const now = Date.now();
  return {
    id: makeId("session"),
    title: "新会话",
    createdAt: now,
    updatedAt: now,
    provider: settings.provider,
    model: currentModelFor(settings),
    messages: [],
    sessionObjects: [],
    demoLog: [],
    lastValidation: null,
    lastDrawParams: null,
  };
}

function sessionTitleFromMessages(messages: Message[]) {
  const lastUserMessage = [...messages].reverse().find((message) => message.role === "user");
  if (!lastUserMessage || !("content" in lastUserMessage)) return "新会话";
  return lastUserMessage.content.replace(/\s+/g, " ").trim().slice(0, 28) || "新会话";
}

function normalizeChatSession(value: Partial<ChatSession>, fallback: ChatSession): ChatSession {
  const provider = normalizeProvider(value.provider ?? fallback.provider);
  return {
    id: typeof value.id === "string" && value.id ? value.id : fallback.id,
    title: typeof value.title === "string" && value.title ? value.title : fallback.title,
    createdAt: Number(value.createdAt || fallback.createdAt),
    updatedAt: Number(value.updatedAt || fallback.updatedAt),
    provider,
    model: typeof value.model === "string" && value.model ? value.model : fallback.model,
    messages: Array.isArray(value.messages) ? (value.messages as Message[]) : [],
    sessionObjects: Array.isArray(value.sessionObjects)
      ? (value.sessionObjects as SessionObject[])
      : [],
    demoLog: Array.isArray(value.demoLog) ? (value.demoLog as DemoLogEntry[]) : [],
    lastValidation: value.lastValidation ?? null,
    lastDrawParams: value.lastDrawParams ?? null,
  };
}

function loadChatSessions(settings: SettingsView): StoredChatSessions {
  const fallback = createChatSession(settings);
  if (typeof window === "undefined") {
    return { activeSessionId: fallback.id, sessions: [fallback] };
  }

  try {
    const raw = window.localStorage.getItem(CHAT_SESSIONS_STORAGE_KEY);
    if (!raw) return { activeSessionId: fallback.id, sessions: [fallback] };
    const parsed = JSON.parse(raw) as Partial<StoredChatSessions>;
    const sessions = Array.isArray(parsed.sessions)
      ? parsed.sessions
          .map((session) => normalizeChatSession(session, fallback))
          .sort((a, b) => b.updatedAt - a.updatedAt)
          .slice(0, 30)
      : [];
    if (sessions.length === 0) {
      return { activeSessionId: fallback.id, sessions: [fallback] };
    }
    const activeSessionId =
      typeof parsed.activeSessionId === "string" &&
      sessions.some((session) => session.id === parsed.activeSessionId)
        ? parsed.activeSessionId
        : sessions[0].id;
    return { activeSessionId, sessions };
  } catch {
    return { activeSessionId: fallback.id, sessions: [fallback] };
  }
}

function normalizeGlassSettings(value: Partial<GlassSettings> & { opacity?: number }): GlassSettings {
  return {
    transparency: clamp(
      Number(value.transparency ?? value.opacity ?? DEFAULT_GLASS_SETTINGS.transparency),
      0,
      90
    ),
    blur: clamp(Number(value.blur ?? DEFAULT_GLASS_SETTINGS.blur), 0, 100),
    border: value.border === "glow" ? "glow" : "pixel",
  };
}

function glassCssVariables(settings: GlassSettings, fontSize?: number): CSSProperties {
  const next = normalizeGlassSettings(settings);
  const transparency = clamp(next.transparency, 0, 90);
  // 0 = opaque, 1 = fully transparent
  const transparencyRatio = transparency / 90;
  // aggressive curve: at 90% transparency most layers approach 0
  const transparencyCurve = Math.pow(transparencyRatio, 0.85);
  // 0 = clear glass, 1 = heavy frost
  const roughness = clamp(next.blur / 100, 0, 1);
  const roughnessCurve = Math.pow(roughness, 1.4);

  // Alpha layers: each decays at a different rate so controls stay visible
  // even when the background is nearly gone.
  const glassAlpha = clamp(1 - transparencyCurve, 0, 1);
  const glassAlphaSoft = clamp(1 - transparencyCurve * 0.9, 0, 1);
  const glassAlphaMuted = clamp(1 - transparencyCurve * 0.85, 0, 1);
  const controlAlpha = clamp(1 - transparencyCurve * 0.5, 0, 1);

  const blurPx = Math.round(roughnessCurve * 42);
  const grainStep = Math.round(clamp(14 - roughnessCurve * 8, 6, 14));

  // Decorative layers (grid lines, gradient edges) also fade with transparency.
  const gridAlpha = (0.1 * clamp(1 - transparencyCurve * 0.9, 0, 1)).toFixed(3);
  const gridAlphaSubtle = (0.08 * clamp(1 - transparencyCurve * 0.9, 0, 1)).toFixed(3);
  // Gradient edges: fully transparent at 90%.
  const edgeAlpha = clamp(1 - transparencyCurve, 0, 1);
  // Text shadow strengthens as the panel becomes transparent —
  // white text on a transparent dark panel needs a dark halo to stay readable.
  const textShadowStrength = clamp(1 - glassAlpha, 0, 0.75);

  return {
    ...(typeof fontSize === "number" ? { "--content-font-size": `${fontSize}px` } : {}),
    "--window-bg-alpha": glassAlpha.toFixed(2),
    "--glass-alpha": glassAlpha.toFixed(2),
    "--glass-alpha-soft": glassAlphaSoft.toFixed(2),
    "--glass-alpha-muted": glassAlphaMuted.toFixed(2),
    "--control-alpha": controlAlpha.toFixed(2),
    "--glass-blur": `${blurPx}px`,
    "--glass-roughness": roughness.toFixed(2),
    "--glass-frost-alpha": clamp(roughnessCurve * 0.45, 0, 0.45).toFixed(2),
    "--glass-grain-light": clamp(roughnessCurve * 0.35, 0, 0.35).toFixed(2),
    "--glass-grain-dark": clamp(roughnessCurve * 0.18, 0, 0.18).toFixed(2),
    "--glass-grain-step": `${grainStep}px`,
    "--glass-shine-opacity": clamp(1 - roughnessCurve * 0.55, 0.45, 1).toFixed(2),
    "--ambient-opacity": clamp(1 - transparencyCurve * 0.7, 0.05, 1).toFixed(2),
    "--glass-saturation-strong": `${Math.round(190 - roughnessCurve * 60)}%`,
    "--glass-saturation": `${Math.round(180 - roughnessCurve * 55)}%`,
    "--glass-saturation-soft": `${Math.round(165 - roughnessCurve * 45)}%`,
    "--glass-saturation-muted": `${Math.round(155 - roughnessCurve * 40)}%`,
    "--grid-alpha": gridAlpha,
    "--grid-alpha-subtle": gridAlphaSubtle,
    "--edge-alpha": edgeAlpha.toFixed(2),
    "--text-shadow-strength": textShadowStrength.toFixed(2),
  } as CSSProperties;
}

function saveGlassSettingsNow(settings: GlassSettings) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(GLASS_STORAGE_KEY, JSON.stringify(normalizeGlassSettings(settings)));
}

function applyGlassCssVariables(settings: GlassSettings) {
  if (typeof document === "undefined") return;
  const app = document.querySelector<HTMLElement>(".cadegg-app");
  if (!app) return;
  const vars = glassCssVariables(settings);
  Object.entries(vars).forEach(([name, value]) => {
    app.style.setProperty(name, String(value));
  });
}

function normalizeAppPreferences(value: Partial<AppPreferences>): AppPreferences {
  return {
    language: value.language === "en-US" ? "en-US" : "zh-CN",
    fontSize: clamp(Number(value.fontSize ?? DEFAULT_APP_PREFERENCES.fontSize), 12, 18),
    storageLocation: value.storageLocation === "project" ? "project" : "appdata",
    notifications: value.notifications ?? DEFAULT_APP_PREFERENCES.notifications,
    autoSyncObjects: value.autoSyncObjects ?? DEFAULT_APP_PREFERENCES.autoSyncObjects,
    alwaysOnTop: value.alwaysOnTop ?? DEFAULT_APP_PREFERENCES.alwaysOnTop,
    reduceMotion: value.reduceMotion ?? DEFAULT_APP_PREFERENCES.reduceMotion,
    densePanels: value.densePanels ?? DEFAULT_APP_PREFERENCES.densePanels,
  };
}

function loadGlassSettings(): GlassSettings {
  if (typeof window === "undefined") return DEFAULT_GLASS_SETTINGS;

  try {
    const raw = window.localStorage.getItem(GLASS_STORAGE_KEY);
    if (!raw) return DEFAULT_GLASS_SETTINGS;
    const parsed = JSON.parse(raw) as Partial<GlassSettings> & { opacity?: number };

    return normalizeGlassSettings(parsed);
  } catch {
    return DEFAULT_GLASS_SETTINGS;
  }
}

function loadAppPreferences(): AppPreferences {
  if (typeof window === "undefined") return DEFAULT_APP_PREFERENCES;

  try {
    const raw = window.localStorage.getItem(APP_PREFS_STORAGE_KEY);
    if (!raw) return DEFAULT_APP_PREFERENCES;
    const parsed = JSON.parse(raw) as Partial<AppPreferences>;

    return normalizeAppPreferences(parsed);
  } catch {
    return DEFAULT_APP_PREFERENCES;
  }
}

export default function App() {
  const appWindowRef = useRef<ReturnType<typeof getCurrentWindow> | null>(null);
  if (appWindowRef.current === null) {
    appWindowRef.current = getCurrentWindow();
  }
  const appWindow = appWindowRef.current;
  const [view, setView] = useState<View>("chat");
  const [glassSettings, setGlassSettings] = useState<GlassSettings>(() => loadGlassSettings());
  const [appPreferences, setAppPreferences] = useState<AppPreferences>(() => loadAppPreferences());
  const appPreferencesRef = useRef(appPreferences);
  const initialSessionsRef = useRef<StoredChatSessions | null>(null);
  if (initialSessionsRef.current === null) {
    initialSessionsRef.current = loadChatSessions(DEFAULT_VIEW);
  }
  const initialSessions = initialSessionsRef.current;
  const initialSession =
    initialSessions.sessions.find((session) => session.id === initialSessions.activeSessionId) ??
    initialSessions.sessions[0];
  const [settings, setSettings] = useState<SettingsView>({
    ...DEFAULT_VIEW,
    provider: initialSession.provider,
    [providerMeta(initialSession.provider).strongModelField]: initialSession.model,
  });
  const [sessions, setSessions] = useState<ChatSession[]>(initialSessions.sessions);
  const [activeSessionId, setActiveSessionId] = useState(initialSession.id);

  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<Message[]>(initialSession.messages);
  const [assistantDraft, setAssistantDraft] = useState("");
  const [sessionObjects, setSessionObjects] = useState<SessionObject[]>(
    initialSession.sessionObjects
  );
  const [sending, setSending] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const [savedHint, setSavedHint] = useState(false);
  const [glmKeyDraft, setGlmKeyDraft] = useState<string | null>(null);
  const [deepseekKeyDraft, setDeepseekKeyDraft] = useState<string | null>(null);
  const [qwenKeyDraft, setQwenKeyDraft] = useState<string | null>(null);
  const [kimiKeyDraft, setKimiKeyDraft] = useState<string | null>(null);

  const [testStatus, setTestStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [syncingObjects, setSyncingObjects] = useState(false);
  const [importingSelection, setImportingSelection] = useState(false);

  const [demoLog, setDemoLog] = useState<DemoLogEntry[]>(initialSession.demoLog);
  const [lastValidation, setLastValidation] = useState<ElevatorValidation | null>(
    initialSession.lastValidation
  );
  const [lastDrawParams, setLastDrawParams] = useState<Record<string, unknown> | null>(
    initialSession.lastDrawParams
  );

  const scrollRef = useRef<HTMLDivElement | null>(null);
  const assistantDraftRef = useRef("");
  const sessionObjectsRef = useRef<SessionObject[]>(initialSession.sessionObjects);
  const pendingUndoSnapshotRef = useRef<SessionObject[] | null>(null);
  const undoSnapshotsRef = useRef<SessionObject[][]>([]);
  const runTouchedObjectTableRef = useRef(false);
  const pendingPostRunSyncRef = useRef(false);
  const pendingToolCallsRef = useRef<Record<string, ToolCall>>({});
  const completedToolIdsRef = useRef<Set<string>>(new Set());
  const [completedToolIds, setCompletedToolIds] = useState<Set<string>>(new Set());
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
      // Tool output is often human-readable text. Ignore non-JSON validation payloads.
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
      setSettings((prev) => {
        const next = normalizeSettingsView(s);
        const active = sessions.find((session) => session.id === activeSessionId);
        if (!active) return next;
        const meta = providerMeta(active.provider);
        return {
          ...next,
          provider: active.provider,
          [meta.strongModelField]: active.model || prev[meta.strongModelField],
        };
      });
      setGlmKeyDraft(null);
      setDeepseekKeyDraft(null);
      setQwenKeyDraft(null);
      setKimiKeyDraft(null);
    } catch (e) {
      console.error("load settings:", e);
    }
  }

  useEffect(() => {
    refreshSettings();
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      window.localStorage.setItem(GLASS_STORAGE_KEY, JSON.stringify(glassSettings));
    }, 180);
    return () => window.clearTimeout(timer);
  }, [glassSettings]);

  // Native Effect.Blur on Windows makes the window opaque, overriding CSS transparency.
  // CSS backback-filter on panels is sufficient for the glass effect.
  // Apply CSS variables from stored settings once on mount.
  useEffect(() => {
    const stored = loadGlassSettings();
    applyGlassCssVariables(stored);
  }, []);

  useEffect(() => {
    appPreferencesRef.current = appPreferences;
    const timer = window.setTimeout(() => {
      window.localStorage.setItem(APP_PREFS_STORAGE_KEY, JSON.stringify(appPreferences));
    }, 180);
    return () => window.clearTimeout(timer);
  }, [appPreferences]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      window.localStorage.setItem(
        CHAT_SESSIONS_STORAGE_KEY,
        JSON.stringify({ activeSessionId, sessions })
      );
    }, 180);
    return () => window.clearTimeout(timer);
  }, [activeSessionId, sessions]);

  useEffect(() => {
    const now = Date.now();
    const title = sessionTitleFromMessages(messages);
    const provider = settings.provider;
    const model = currentModelFor(settings, provider);
    setSessions((prev) =>
      prev
        .map((session) =>
          session.id === activeSessionId
            ? {
                ...session,
                title,
                updatedAt: now,
                provider,
                model,
                messages,
                sessionObjects,
                demoLog,
                lastValidation,
                lastDrawParams,
              }
            : session
        )
        .sort((a, b) => b.updatedAt - a.updatedAt)
    );
  }, [
    activeSessionId,
    messages,
    sessionObjects,
    demoLog,
    lastValidation,
    lastDrawParams,
    settings.provider,
    settings.glm_strong_model,
    settings.deepseek_strong_model,
    settings.qwen_strong_model,
    settings.kimi_strong_model,
  ]);

  useEffect(() => {
    void appWindow.setAlwaysOnTop(appPreferences.alwaysOnTop).catch((e) => {
      console.error("set always on top:", e);
    });
  }, [appWindow, appPreferences.alwaysOnTop]);

  // Subscribe to agent streaming events from the Rust backend.
  // listen() resolves async; React StrictMode can clean up before the promise resolves.
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
        if (
          e.tool_calls.length > 0 ||
          (e.text && assistantDraftRef.current && e.text === assistantDraftRef.current)
        ) {
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
        completedToolIdsRef.current.add(e.result.id);
        setCompletedToolIds(new Set(completedToolIdsRef.current));
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
          if (e.result.ok) {
            const call = pendingToolCallsRef.current[e.result.id];
            if (call) setLastDrawParams(call.args);
          }
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
          pendingPostRunSyncRef.current = appPreferencesRef.current.autoSyncObjects;
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
  }, [messages, assistantDraft, sending]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.shiftKey && e.key === "0") {
        setGlassSettings(DEFAULT_GLASS_SETTINGS);
        saveGlassSettingsNow(DEFAULT_GLASS_SETTINGS);
        setAppPreferences(DEFAULT_APP_PREFERENCES);
        setErrorMsg("外观与字体设置已重置");
      } else if (e.key === "Escape" && view === "settings") {
        setView("chat");
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [view]);

  async function handleSubmit(e: FormEvent) {
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
    completedToolIdsRef.current = new Set();
    setCompletedToolIds(new Set());

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

      // run_agent emits agent:event for each step; resolve only means the backend loop ended.
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

  async function saveSettings(nextSettings: SettingsView = settings) {
    try {
      await invoke("save_settings", {
        update: {
          provider: nextSettings.provider,
          work_mode: nextSettings.work_mode,
          glm_model: nextSettings.glm_model,
          glm_strong_model: nextSettings.glm_strong_model,
          glm_base_url: nextSettings.glm_base_url,
          deepseek_model: nextSettings.deepseek_model,
          deepseek_strong_model: nextSettings.deepseek_strong_model,
          deepseek_base_url: nextSettings.deepseek_base_url,
          qwen_model: nextSettings.qwen_model,
          qwen_strong_model: nextSettings.qwen_strong_model,
          qwen_base_url: nextSettings.qwen_base_url,
          kimi_model: nextSettings.kimi_model,
          kimi_strong_model: nextSettings.kimi_strong_model,
          kimi_base_url: nextSettings.kimi_base_url,
          glm_api_key: glmKeyDraft,
          deepseek_api_key: deepseekKeyDraft,
          qwen_api_key: qwenKeyDraft,
          kimi_api_key: kimiKeyDraft,
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
        prev
          .map((message, index) =>
            index === messageIndex && message.role === "tool"
              ? { ...message, confirmed: true }
              : message
          )
          .concat({ role: "tool", ...result })
      );

      if (result.ok && shouldAutoSyncObjectTable(result.name)) {
        runTouchedObjectTableRef.current = true;
      }
      if (result.object_updates.length > 0) {
        updateSessionObjects((prev) => applyObjectUpdates(prev, result.object_updates));
      }
      if (result.ok && shouldAutoSyncObjectTable(result.name) && appPreferencesRef.current.autoSyncObjects) {
        await syncSessionObjects(false);
      }
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setSending(false);
    }
  }

  function applySession(session: ChatSession) {
    assistantDraftRef.current = "";
    pendingToolCallsRef.current = {};
    completedToolIdsRef.current = new Set();
    setCompletedToolIds(new Set());
    pendingLogRef.current = null;
    pendingUndoSnapshotRef.current = null;
    runTouchedObjectTableRef.current = false;
    pendingPostRunSyncRef.current = false;
    sessionObjectsRef.current = session.sessionObjects;
    setActiveSessionId(session.id);
    setMessages(session.messages);
    setAssistantDraft("");
    setSessionObjects(session.sessionObjects);
    setDemoLog(session.demoLog);
    setLastValidation(session.lastValidation);
    setLastDrawParams(session.lastDrawParams);
    setInput("");
    setErrorMsg(null);
    const meta = providerMeta(session.provider);
    const nextSettings = {
      ...settings,
      provider: session.provider,
      [meta.strongModelField]: session.model || settings[meta.strongModelField],
    };
    setSettings(nextSettings);
    void saveSettings(nextSettings).catch((e) => setErrorMsg(String(e)));
  }

  function handleNewConversation() {
    if (sending) return;
    const session = createChatSession(settings);
    setSessions((prev) => [session, ...prev]);
    applySession(session);
  }

  function handleSelectSession(id: string) {
    if (sending || id === activeSessionId) return;
    const session = sessions.find((item) => item.id === id);
    if (session) applySession(session);
  }

  function handleDeleteSession(id: string) {
    if (sending) return;
    const remaining = sessions.filter((session) => session.id !== id);
    if (remaining.length > 0) {
      setSessions(remaining);
      if (id === activeSessionId) applySession(remaining[0]);
      return;
    }
    const next = createChatSession(settings);
    setSessions([next]);
    applySession(next);
  }

  async function handleModelChange(provider: Provider, model: string) {
    const meta = providerMeta(provider);
    const nextSettings = {
      ...settings,
      provider,
      [meta.strongModelField]: model,
    };
    setSettings(nextSettings);
    setSessions((prev) =>
      prev.map((s) =>
        s.id === activeSessionId ? { ...s, provider, model, updatedAt: Date.now() } : s,
      ),
    );
    await saveSettings(nextSettings);
  }

  async function recoverWindow(message?: string) {
    try {
      await appWindow.unminimize();
      await appWindow.show();
      await appWindow.center();
      await appWindow.setFocus();
      if (message) setErrorMsg(message);
    } catch (e) {
      setErrorMsg(`窗口恢复失败：${String(e)}`);
    }
  }

  function handleWindowDrag(e: MouseEvent<HTMLElement>) {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest("button, input, textarea, select, a, [data-no-drag]")) return;
    void appWindow.startDragging().catch((err) => {
      setErrorMsg(`窗口拖拽失败：${String(err)}`);
    });
  }

  async function runWindowAction(action: "minimize" | "toggleMaximize" | "close") {
    try {
      if (action === "minimize") {
        await appWindow.minimize();
      } else if (action === "toggleMaximize") {
        await appWindow.toggleMaximize();
      } else {
        await appWindow.close();
      }
    } catch (e) {
      setErrorMsg(`窗口控制失败：${String(e)}`);
    }
  }

  const selectedProviderMeta = providerMeta(settings.provider);
  const providerLabel = selectedProviderMeta.shortLabel;
  const currentModel = currentModelFor(settings);
  const bridgeState =
    testStatus === null ? "idle" : testStatus.ok ? "online" : "error";
  const bridgeLabel =
    bridgeState === "online" ? "BRIDGE 在线" : bridgeState === "error" ? "BRIDGE 异常" : "BRIDGE 待测";
  const objectReferenceHints = getObjectReferenceHints(sessionObjects);
  const currentKeySet = Boolean(settings[selectedProviderMeta.keySetField]);
  const quickPrompts = [
    "画一条 7000mm 的直线",
    "画一个半径 3000 的圆",
    "画一个双跑楼梯，层高 3000",
    "画一个电梯井口防护门，井口宽 2000，高 1800",
  ];
  const sessionTitle =
    sessions.find((session) => session.id === activeSessionId)?.title ??
    sessionTitleFromMessages(messages);
  const glassStyle = glassCssVariables(glassSettings, appPreferences.fontSize);

  return (
    <div
      className={`cadegg-app ${glassSettings.border === "glow" ? "glass-glow" : "glass-pixel"} ${
        appPreferences.reduceMotion ? "reduce-motion" : ""
      } ${appPreferences.densePanels ? "dense-panels" : ""}`}
      style={glassStyle}
    >
      <div className="ambient-layer" aria-hidden="true" />

      <header className="topbar glass-dark">
        <div className="brand-lockup drag-zone" onMouseDown={handleWindowDrag}>
          <EggLogo />
          <div>
            <div className="brand-name">CADEgg</div>
            <div className="brand-caption">AutoCAD AGENT</div>
          </div>
        </div>

        <div className="session-title drag-zone" title={sessionTitle} onMouseDown={handleWindowDrag}>
          {sessionTitle}
        </div>

        <div className="topbar-actions" data-no-drag>
          <StatusPill state={bridgeState} label={bridgeLabel} />
          <div className="model-chip" title={`${providerLabel} · ${currentModel}`} data-no-drag>
            {currentModel || providerLabel}
          </div>
          <div className="window-controls" data-no-drag onMouseDown={(e) => e.stopPropagation()}>
            <button type="button" onClick={() => void runWindowAction("minimize")} aria-label="最小化">
              <span />
            </button>
            <button type="button" onClick={() => void runWindowAction("toggleMaximize")} aria-label="最大化">
              <span />
            </button>
            <button type="button" onClick={() => void runWindowAction("close")} aria-label="关闭">
              <span />
            </button>
          </div>
        </div>
      </header>

      <div className="decor-stripe" aria-hidden="true">
        <span />
        <span />
      </div>

      <div className="app-grid">
        <aside className="sidebar glass-panel pink-glass">
          <button type="button" className="new-session-button" onClick={handleNewConversation}>
            <IconPlus />
            <span>新建会话</span>
          </button>

          <div className="sidebar-section">
            <div className="section-label">会话</div>
            {sessions.map((session) => (
              <button
                type="button"
                className={`session-card ${session.id === activeSessionId ? "active" : ""}`}
                onClick={() => handleSelectSession(session.id)}
                key={session.id}
              >
                <strong>{session.title}</strong>
                <span>
                  {providerMeta(session.provider).shortLabel} · {session.model || "默认模型"}
                </span>
                {sessions.length > 1 && (
                  <i
                    role="button"
                    tabIndex={0}
                    aria-label="删除会话"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDeleteSession(session.id);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        e.stopPropagation();
                        handleDeleteSession(session.id);
                      }
                    }}
                  >
                    ×
                  </i>
                )}
              </button>
            ))}
          </div>

          <div className="sidebar-spacer" />

          <button type="button" className="sidebar-settings" onClick={() => setView("settings")}>
            <IconGear />
            <span>设置</span>
            <span>v0.3.4</span>
          </button>
        </aside>

        <section className="chat-column">
          <div ref={scrollRef} className="message-stage">
            {messages.length === 0 ? (
              <WelcomeStage
                quickPrompts={quickPrompts}
                setInput={setInput}
                currentKeySet={currentKeySet}
                providerLabel={providerLabel}
                onOpenSettings={() => setView("settings")}
              />
            ) : (
              messages.map((m, i) => renderMessage(m, i, handleConfirmToolCall, completedToolIds))
            )}

            {sending && (
              <div className="agent-status">
                {assistantDraft ? (
                  <div className="message-bubble assistant-message streaming">
                    <span className="streaming-cursor" />
                    {assistantDraft}
                  </div>
                ) : (
                  <div className="typing-indicator" aria-label="正在分析">
                    <span />
                    <span />
                    <span />
                    <b>正在分析...</b>
                  </div>
                )}
              </div>
            )}

            {errorMsg && <div className="inline-error">{errorMsg}</div>}
          </div>

          <form className="composer glass-panel amber-glass" onSubmit={handleSubmit}>
            <ModelPicker
              settings={settings}
              currentKeySet={currentKeySet}
              onModelChange={handleModelChange}
              onOpenSettings={() => setView("settings")}
            />
            <div className="composer-hint">Enter 发送 · Shift+Enter 换行</div>
            <textarea
              rows={2}
              placeholder={
                sending
                  ? "等待回复..."
                  : "描述你要绘制、修改或查询的 CAD 操作..."
              }
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSubmit(e as unknown as FormEvent);
                }
              }}
              disabled={sending}
            />
            <div className="composer-side">
              <span>{providerLabel}</span>
              <button type="submit" disabled={!input.trim() || sending} aria-label="发送">
                <IconSend />
              </button>
            </div>
          </form>
        </section>

        <aside className="right-rail glass-panel lavender-glass">
          <CadCard
            testStatus={testStatus}
            undoing={undoing}
            sending={sending}
            syncingObjects={syncingObjects}
            runCadAction={runCadAction}
            handleUndoLastGeneration={handleUndoLastGeneration}
          />
          <DrawResultCard lastDrawParams={lastDrawParams} />
          <ValidationCard lastValidation={lastValidation} />
          <SessionObjectsCard
            sessionObjects={sessionObjects}
            objectReferenceHints={objectReferenceHints}
            sending={sending}
            undoing={undoing}
            syncingObjects={syncingObjects}
            importingSelection={importingSelection}
            handleImportSelectedObjects={handleImportSelectedObjects}
            syncSessionObjects={syncSessionObjects}
          />
        </aside>
      </div>

      {view === "settings" && (
        <SettingsModal
          settings={settings}
          setSettings={setSettings}
          appPreferences={appPreferences}
          setAppPreferences={setAppPreferences}
          glassSettings={glassSettings}
          setGlassSettings={setGlassSettings}
          savedHint={savedHint}
          saveSettings={saveSettings}
          glmKeyDraft={glmKeyDraft}
          setGlmKeyDraft={setGlmKeyDraft}
          deepseekKeyDraft={deepseekKeyDraft}
          setDeepseekKeyDraft={setDeepseekKeyDraft}
          qwenKeyDraft={qwenKeyDraft}
          setQwenKeyDraft={setQwenKeyDraft}
          kimiKeyDraft={kimiKeyDraft}
          setKimiKeyDraft={setKimiKeyDraft}
          onRecoverWindow={recoverWindow}
          onClose={() => setView("chat")}
        />
      )}
    </div>
  );
}

function ModelPicker({
  settings,
  currentKeySet,
  onModelChange,
  onOpenSettings,
}: {
  settings: SettingsView;
  currentKeySet: boolean;
  onModelChange: (provider: Provider, model: string) => Promise<void>;
  onOpenSettings: () => void;
}) {
  const meta = providerMeta(settings.provider);
  const model = currentModelFor(settings);
  const hasPreset = meta.models.some((item) => item.id === model);

  return (
    <div className="model-picker" data-no-drag>
      <select
        aria-label="模型供应商"
        value={settings.provider}
        onChange={(e) => {
          const provider = e.target.value as Provider;
          const nextMeta = providerMeta(provider);
          const nextModel = String(settings[nextMeta.strongModelField] || nextMeta.models[0].id);
          void onModelChange(provider, nextModel);
        }}
      >
        {MODEL_PROVIDERS.map((provider) => (
          <option key={provider.id} value={provider.id}>
            {provider.label}
          </option>
        ))}
      </select>

      <select
        aria-label="当前会话模型"
        value={hasPreset ? model : "__custom"}
        onChange={(e) => {
          if (e.target.value !== "__custom") {
            void onModelChange(settings.provider, e.target.value);
          }
        }}
      >
        {!hasPreset && <option value="__custom">{model}</option>}
        {meta.models.map((item) => (
          <option key={item.id} value={item.id}>
            {item.label}
          </option>
        ))}
      </select>

      <button
        type="button"
        className={`key-status ${currentKeySet ? "ready" : ""}`}
        onClick={onOpenSettings}
      >
        {currentKeySet ? "BYOK 已配置" : "填写 Key"}
      </button>
      <span className="failover-chip">自动轮转</span>
    </div>
  );
}

function WelcomeStage({
  quickPrompts,
  setInput,
  currentKeySet,
  providerLabel,
  onOpenSettings,
}: {
  quickPrompts: string[];
  setInput: (value: string) => void;
  currentKeySet: boolean;
  providerLabel: string;
  onOpenSettings: () => void;
}) {
  return (
    <div className="welcome">
      <div className="hero-logo">
        <EggLogo large />
      </div>
      <h1>CADEgg</h1>
      <p>选择模型后，直接描述你要在 AutoCAD 里完成的操作。</p>

      {!currentKeySet && (
        <button type="button" className="key-warning" onClick={onOpenSettings}>
          <b>需要配置 {providerLabel} API Key</b>
          <span>点这里打开设置</span>
        </button>
      )}

      <div className="prompt-list">
        {quickPrompts.map((prompt) => (
          <button type="button" key={prompt} onClick={() => setInput(prompt)}>
            {prompt}
          </button>
        ))}
      </div>
    </div>
  );
}

function CadCard({
  testStatus,
  undoing,
  sending,
  syncingObjects,
  runCadAction,
  handleUndoLastGeneration,
}: {
  testStatus: { ok: boolean; msg: string } | null;
  undoing: boolean;
  sending: boolean;
  syncingObjects: boolean;
  runCadAction: (cmd: "test_cad_connection" | "draw_test_line") => Promise<void>;
  handleUndoLastGeneration: () => Promise<void>;
}) {
  return (
    <section className="rail-card cad-card">
      <PanelHeader title="CAD 连接" status={testStatus?.ok ? "online" : "idle"} />
      <p>桥接优先 · COM 回退</p>
      <div className="button-row">
        <button type="button" onClick={() => runCadAction("test_cad_connection")}>
          连接
        </button>
        <button type="button" onClick={() => runCadAction("draw_test_line")}>
          画线
        </button>
      </div>
      <button
        type="button"
        className="dark-action"
        onClick={handleUndoLastGeneration}
        disabled={sending || undoing || syncingObjects}
      >
        {undoing ? "撤回中..." : "撤回上一次生成"}
      </button>
      {testStatus && (
        <div className={`status-readout ${testStatus.ok ? "ok" : "bad"}`}>{testStatus.msg}</div>
      )}
    </section>
  );
}

function DrawResultCard({
  lastDrawParams,
}: {
  lastDrawParams: Record<string, unknown> | null;
}) {
  if (!lastDrawParams) {
    return (
      <section className="rail-card muted-card">
        <PanelHeader title="本次出图" />
        <p>等待 AutoCAD 生成结果</p>
      </section>
    );
  }

  return (
    <section className="rail-card">
      <PanelHeader title="本次出图" />
      <div className="metric-grid">
        <Metric label="井口宽度" value={`${String(lastDrawParams.opening_width ?? "-")} mm`} />
        <Metric label="井口高度" value={`${String(lastDrawParams.opening_height ?? "-")} mm`} />
        <Metric label="防护门高" value={`${String(lastDrawParams.guard_height ?? "1500")} mm`} />
        <Metric label="踢脚板" value={`${String(lastDrawParams.toe_board_height ?? "200")} mm`} />
      </div>
      <div className="tag-row">
        <span>警示牌 {lastDrawParams.include_warning_sign === false ? "未配" : "已配"}</span>
        <span>材料表 {lastDrawParams.include_material_table === false ? "未配" : "已配"}</span>
      </div>
    </section>
  );
}

function ValidationCard({
  lastValidation,
}: {
  lastValidation: ElevatorValidation | null;
}) {
  if (!lastValidation) {
    return (
      <section className="rail-card muted-card">
        <PanelHeader title="安全校核" />
        <p>校核结果会在工具执行后出现</p>
      </section>
    );
  }

  return (
    <section className={`rail-card validation-card ${lastValidation.ok ? "valid" : "invalid"}`}>
      <PanelHeader title="安全校核" status={lastValidation.ok ? "online" : "error"} />
      <strong>{lastValidation.ok ? "安全校核通过" : "安全校核未通过"}</strong>
      <div className="check-list">
        {lastValidation.checks.map((check) => (
          <div key={check.id}>
            <span className={check.passed ? "pass" : "fail"}>
              {check.passed ? <IconCheck /> : <IconCross />}
            </span>
            <span>{check.label}</span>
          </div>
        ))}
      </div>
      {lastValidation.issues.length > 0 && (
        <div className="risk-box">风险项：{lastValidation.issues.join("；")}</div>
      )}
      <p>
        材料表：防护门 {lastValidation.material_table.guard_door} · 踢脚板{" "}
        {lastValidation.material_table.toe_board_height}mm · 警示牌{" "}
        {lastValidation.material_table.warning_sign ? "已配" : "未配"}
      </p>
    </section>
  );
}

function SessionObjectsCard({
  sessionObjects,
  objectReferenceHints,
  sending,
  undoing,
  syncingObjects,
  importingSelection,
  handleImportSelectedObjects,
  syncSessionObjects,
}: {
  sessionObjects: SessionObject[];
  objectReferenceHints: Map<string, string[]>;
  sending: boolean;
  undoing: boolean;
  syncingObjects: boolean;
  importingSelection: boolean;
  handleImportSelectedObjects: () => Promise<void>;
  syncSessionObjects: (showStatus: boolean) => Promise<SessionObject[] | null>;
}) {
  return (
    <section className="rail-card object-card">
      <PanelHeader title={`会话对象 · ${sessionObjects.length} 个可引用`} />
      <div className="button-row">
        <button
          type="button"
          onClick={handleImportSelectedObjects}
          disabled={sending || undoing || syncingObjects || importingSelection}
        >
          {importingSelection ? "导入中" : "导入选中"}
        </button>
        <button
          type="button"
          onClick={() => void syncSessionObjects(true)}
          disabled={sending || undoing || syncingObjects || importingSelection || sessionObjects.length === 0}
        >
          {syncingObjects ? "同步中" : "同步"}
        </button>
      </div>

      {sessionObjects.length > 0 ? (
        <div className="object-list">
          {sessionObjects.map((object) => (
            <article key={object.handle} className="object-item">
              <div>
                <code>{object.handle}</code>
                <b>{object.kind}</b>
                <span>{sourceDisplayLabel(object.source)}</span>
              </div>
              <p>{object.label}</p>
              <div className="hint-row">
                {(objectReferenceHints.get(object.handle) ?? []).slice(0, 3).map((hint) => (
                  <span key={hint}>{hint}</span>
                ))}
              </div>
            </article>
          ))}
        </div>
      ) : (
        <p>在 AutoCAD 中选中对象后可导入当前会话。</p>
      )}
    </section>
  );
}

function SettingsModal({
  settings,
  setSettings,
  appPreferences,
  setAppPreferences,
  glassSettings,
  setGlassSettings,
  savedHint,
  saveSettings,
  glmKeyDraft,
  setGlmKeyDraft,
  deepseekKeyDraft,
  setDeepseekKeyDraft,
  qwenKeyDraft,
  setQwenKeyDraft,
  kimiKeyDraft,
  setKimiKeyDraft,
  onRecoverWindow,
  onClose,
}: {
  settings: SettingsView;
  setSettings: Dispatch<SetStateAction<SettingsView>>;
  appPreferences: AppPreferences;
  setAppPreferences: Dispatch<SetStateAction<AppPreferences>>;
  glassSettings: GlassSettings;
  setGlassSettings: Dispatch<SetStateAction<GlassSettings>>;
  savedHint: boolean;
  saveSettings: (nextSettings?: SettingsView) => Promise<void>;
  glmKeyDraft: string | null;
  setGlmKeyDraft: (v: string | null) => void;
  deepseekKeyDraft: string | null;
  setDeepseekKeyDraft: (v: string | null) => void;
  qwenKeyDraft: string | null;
  setQwenKeyDraft: (v: string | null) => void;
  kimiKeyDraft: string | null;
  setKimiKeyDraft: (v: string | null) => void;
  onRecoverWindow: (message?: string) => Promise<void>;
  onClose: () => void;
}) {
  const [draftAppPreferences, setDraftAppPreferences] = useState<AppPreferences>(() =>
    normalizeAppPreferences(appPreferences)
  );
  const [draftGlassSettings, setDraftGlassSettings] = useState<GlassSettings>(() =>
    normalizeGlassSettings(glassSettings)
  );
  const [isPreviewingGlass, setIsPreviewingGlass] = useState(false);
  const previewGlassSettingsRef = useRef(draftGlassSettings);

  useEffect(() => {
    setDraftAppPreferences(normalizeAppPreferences(appPreferences));
  }, [appPreferences]);

  useEffect(() => {
    const next = normalizeGlassSettings(glassSettings);
    previewGlassSettingsRef.current = next;
    setDraftGlassSettings(next);
  }, [glassSettings]);

  async function handleSaveAll() {
    const nextAppPreferences = normalizeAppPreferences(draftAppPreferences);
    setAppPreferences(nextAppPreferences);
    await saveSettings();
  }

  function previewGlassSettings(partial: Partial<GlassSettings>) {
    const next = normalizeGlassSettings({
      ...previewGlassSettingsRef.current,
      ...partial,
    });
    previewGlassSettingsRef.current = next;
    applyGlassCssVariables(next);
    saveGlassSettingsNow(next);
    // CSS-only during drag — native effect only on commit to avoid flicker
  }

  function commitGlassSettings(partial: Partial<GlassSettings>) {
    const next = normalizeGlassSettings({
      ...previewGlassSettingsRef.current,
      ...partial,
    });
    previewGlassSettingsRef.current = next;
    setDraftGlassSettings(next);
    applyGlassCssVariables(next);
    saveGlassSettingsNow(next);
    setGlassSettings(next);
  }

  function keyDraftFor(provider: Provider) {
    if (provider === "deepseek") {
      return {
        draft: deepseekKeyDraft,
        setDraft: setDeepseekKeyDraft,
        placeholder: "sk-...",
      };
    }
    if (provider === "qwen") {
      return {
        draft: qwenKeyDraft,
        setDraft: setQwenKeyDraft,
        placeholder: "sk-...",
      };
    }
    if (provider === "kimi") {
      return {
        draft: kimiKeyDraft,
        setDraft: setKimiKeyDraft,
        placeholder: "sk-...",
      };
    }
    return {
      draft: glmKeyDraft,
      setDraft: setGlmKeyDraft,
      placeholder: "xxxxxxxx.xxxx",
    };
  }

  return (
    <div className={`modal-backdrop ${isPreviewingGlass ? "previewing-glass" : ""}`}>
      <section className="settings-modal glass-modal" role="dialog" aria-modal="true">
        <ModalHeader title="总设置" onClose={onClose} />

        <div className="settings-content">
          <section className="settings-group">
            <GroupHeader title="应用" desc="这些选项只影响 CADEgg 前端体验，不会改动 AutoCAD 图形。" />
            <Field label="界面语言" hint="当前文案以简体中文为主，英文界面作为后续完整本地化入口。">
              <select
                className={inputCls}
                value={draftAppPreferences.language}
                onChange={(e) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    language: e.target.value === "en-US" ? "en-US" : "zh-CN",
                  }))
                }
              >
                <option value="zh-CN">简体中文</option>
                <option value="en-US">English</option>
              </select>
            </Field>

            <Field label="字体大小" hint="拖动时仅预览数值，点击保存后应用到会话文字和输入区。">
              <StableRange
                ariaLabel="字体大小"
                className="inline-slider"
                min={12}
                max={18}
                value={draftAppPreferences.fontSize}
                suffix="px"
                onCommit={(fontSize) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    fontSize,
                  }))
                }
              />
            </Field>

            <Field label="存储位置" hint="模型 Key 和模型配置由 Rust 后端保存；当前使用系统 AppData，避免把密钥写入项目目录。">
              <select
                className={inputCls}
                value={draftAppPreferences.storageLocation}
                onChange={(e) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    storageLocation: e.target.value === "project" ? "project" : "appdata",
                  }))
                }
              >
                <option value="appdata">系统 AppData（推荐）</option>
                <option value="project">项目目录（仅记录偏好，后端暂不迁移密钥）</option>
              </select>
            </Field>

            <div className="switch-grid">
              <SwitchField
                label="通知"
                checked={draftAppPreferences.notifications}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, notifications: checked }))
                }
              />
              <SwitchField
                label="对象自动同步"
                checked={draftAppPreferences.autoSyncObjects}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, autoSyncObjects: checked }))
                }
              />
              <SwitchField
                label="窗口置顶"
                checked={draftAppPreferences.alwaysOnTop}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, alwaysOnTop: checked }))
                }
              />
              <SwitchField
                label="减少动画"
                checked={draftAppPreferences.reduceMotion}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, reduceMotion: checked }))
                }
              />
              <SwitchField
                label="紧凑右栏"
                checked={draftAppPreferences.densePanels}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, densePanels: checked }))
                }
              />
            </div>
          </section>

          <section className="settings-group">
            <GroupHeader
              title="外观玻璃"
              desc="拖动会即时应用并保存；透明度控制透出量，粗糙度控制磨砂感和透后清晰度。"
            />
            <StableRange
              label="透明度"
              min={0}
              max={90}
              value={draftGlassSettings.transparency}
              suffix="%"
              onPreview={(transparency) => previewGlassSettings({ transparency })}
              onCommit={(transparency) => commitGlassSettings({ transparency })}
              onDragStateChange={setIsPreviewingGlass}
            />

            <StableRange
              label="粗糙度"
              min={0}
              max={100}
              value={draftGlassSettings.blur}
              suffix="%"
              onPreview={(blur) => previewGlassSettings({ blur })}
              onCommit={(blur) => commitGlassSettings({ blur })}
              onDragStateChange={setIsPreviewingGlass}
            />

            <div className="border-style-field">
              <span>边框样式</span>
              <div className="segmented">
                <button
                  type="button"
                  className={draftGlassSettings.border === "pixel" ? "active" : ""}
                  onClick={() => commitGlassSettings({ border: "pixel" })}
                >
                  像素墨线
                </button>
                <button
                  type="button"
                  className={draftGlassSettings.border === "glow" ? "active" : ""}
                  onClick={() => commitGlassSettings({ border: "glow" })}
                >
                  柔和发光
                </button>
              </div>
            </div>

            <button
              type="button"
              className="outline-action reset-glass"
              onClick={() => commitGlassSettings(DEFAULT_GLASS_SETTINGS)}
            >
              重置玻璃参数
            </button>
            <button
              type="button"
              className="outline-action reset-glass"
              onClick={() => void onRecoverWindow("窗口已重新居中")}
            >
              恢复窗口位置
            </button>
          </section>

          <section className="settings-group">
            <GroupHeader
              title="模型密钥与轮转"
              desc="会话里选择当前模型；这里管理各供应商 BYOK、Base URL 和自动轮转候选。"
            />
            <div className="provider-settings-list">
              {MODEL_PROVIDERS.map((provider) => {
                const keyDraft = keyDraftFor(provider.id);
                const baseUrl = String(settings[provider.baseUrlField] ?? "");
                const cheapModel = String(settings[provider.cheapModelField] ?? "");
                const strongModel = String(settings[provider.strongModelField] ?? "");
                const keySet = Boolean(settings[provider.keySetField]);
                const keyPreview = String(settings[provider.keyPreviewField] ?? "");
                const datalistId = `${provider.id}-models`;

                return (
                  <section className="provider-settings-card" key={provider.id}>
                    <GroupHeader
                      title={provider.label}
                      desc="轻量模型用于普通问答；强模型用于规划、出图、校核。请求失败时后端会自动切换。"
                    />
                    <KeyField
                      label={provider.apiLabel}
                      isSet={keySet}
                      preview={keyPreview}
                      draft={keyDraft.draft}
                      onDraftChange={keyDraft.setDraft}
                      placeholder={keyDraft.placeholder}
                    />
                    <Field label="API Base URL" hint="兼容 OpenAI /chat/completions 的官方或中转地址。">
                      <input
                        type="text"
                        className={inputCls}
                        value={baseUrl}
                        onChange={(e) =>
                          setSettings({ ...settings, [provider.baseUrlField]: e.target.value })
                        }
                      />
                    </Field>
                    <Field label="轻量模型">
                      <input
                        type="text"
                        list={datalistId}
                        className={inputCls}
                        value={cheapModel}
                        onChange={(e) =>
                          setSettings({ ...settings, [provider.cheapModelField]: e.target.value })
                        }
                      />
                    </Field>
                    <Field label="强模型">
                      <input
                        type="text"
                        list={datalistId}
                        className={inputCls}
                        value={strongModel}
                        onChange={(e) =>
                          setSettings({ ...settings, [provider.strongModelField]: e.target.value })
                        }
                      />
                      <datalist id={datalistId}>
                        {provider.models.map((model) => (
                          <option key={model.id} value={model.id}>
                            {model.label}
                          </option>
                        ))}
                      </datalist>
                    </Field>
                  </section>
                );
              })}
            </div>

            <p className="settings-note">
              API Key 仅保存在本机 AppData/settings.json。界面不会明文回显已保存的 key。
            </p>
          </section>
        </div>

        <div className="modal-footer">
          <span>{savedHint ? "已保存" : ""}</span>
          <button type="button" className="outline-action" onClick={onClose}>
            返回
          </button>
          <button type="button" className="primary-action" onClick={() => void handleSaveAll()}>
            保存应用与模型
          </button>
        </div>
      </section>
    </div>
  );
}

function renderMessage(
  m: Message,
  i: number,
  onConfirmToolCall: (messageIndex: number, call: ToolCall) => void,
  completedToolIds: Set<string>
) {
  if (m.role === "user") {
    return (
      <div key={i} className="message-bubble user-message">
        {m.content}
      </div>
    );
  }

  if (m.role === "plan") {
    return (
      <details key={i} className="message-bubble plan-message" open>
        <summary>
          执行计划 · {m.tool_calls.length} 步 · {planSummary(m.tool_calls)}
        </summary>
        {m.text && <div className="message-text">{m.text}</div>}
        <div className="tool-call-list">
          {m.tool_calls.map((tc) => {
            const done = completedToolIds.has(tc.id);
            return (
              <article key={tc.id} className={done ? "tool-done" : "tool-pending"}>
                <span className="tool-status">{done ? <IconCheck /> : <span className="spinner" />}</span>
                <strong>{tc.name}</strong>
                <code>{compactToolArgs(tc.args)}</code>
              </article>
            );
          })}
        </div>
      </details>
    );
  }

  if (m.role === "assistant") {
    return (
      <div key={i} className="assistant-stack">
        {m.text && <div className="message-bubble assistant-message">{m.text}</div>}
        {m.tool_calls.map((tc) => (
          <article key={tc.id} className="tool-call">
            <strong>{tc.name}</strong>
            <code>{JSON.stringify(tc.args)}</code>
          </article>
        ))}
      </div>
    );
  }

  return (
    <div
      key={i}
      className={`message-bubble tool-message ${
        m.confirmation_required ? "confirm" : m.ok ? "ok" : "bad"
      }`}
    >
      <strong>
        {m.confirmation_required ? "!" : m.ok ? <IconCheck /> : <IconCross />} {m.name}
      </strong>
      <span>{m.content}</span>
      {m.confirmation_required && m.pending_call && !m.confirmed && (
        <button type="button" onClick={() => onConfirmToolCall(i, m.pending_call!)}>
          确认执行
        </button>
      )}
      {m.confirmation_required && m.confirmed && <em>已确认，结果见下方。</em>}
    </div>
  );
}

function PanelHeader({
  title,
  status,
}: {
  title: string;
  status?: "online" | "error" | "idle";
}) {
  return (
    <header className="panel-header">
      <h2>{title}</h2>
      {status && <span className={`led ${status}`} />}
    </header>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <b>{value}</b>
    </div>
  );
}

function StatusPill({ state, label }: { state: "online" | "error" | "idle"; label: string }) {
  return (
    <div className="status-pill">
      <span className={`led ${state}`} />
      <span>{label}</span>
    </div>
  );
}

function ModalHeader({ title, onClose }: { title: string; onClose: () => void }) {
  return (
    <header className="modal-header">
      <h2>{title}</h2>
      <button type="button" className="icon-button" onClick={onClose} aria-label="关闭">
        <IconCross />
      </button>
    </header>
  );
}

const inputCls = "settings-input";

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
      ? "正在修改，保存后覆盖原 key"
      : "已保存。点右侧按钮修改，原 key 不会被读回前端。"
    : "本地保存到 AppData，不上传任何服务器。";

  return (
    <Field label={label} hint={hint}>
      {editing ? (
        <div className="key-edit-row">
          <input
            type="password"
            className={inputCls}
            placeholder={placeholder}
            value={draft ?? ""}
            onChange={(e) => onDraftChange(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            autoFocus
          />
          <button type="button" className="outline-action" onClick={() => onDraftChange(null)}>
            取消
          </button>
        </div>
      ) : (
        <div className="key-edit-row">
          <div className={`${inputCls} key-preview`}>{isSet ? preview : "（未设置）"}</div>
          <button type="button" className="outline-action" onClick={() => onDraftChange("")}>
            {isSet ? "修改" : "设置"}
          </button>
        </div>
      )}
    </Field>
  );
}

function GroupHeader({ title, desc }: { title: string; desc: string }) {
  return (
    <header className="group-header">
      <h3>{title}</h3>
      <p>{desc}</p>
    </header>
  );
}

function SwitchField({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="switch-field">
      <span>{label}</span>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <b aria-hidden="true" />
    </label>
  );
}

function StableRange({
  label,
  ariaLabel,
  className,
  min,
  max,
  value,
  suffix,
  onPreview,
  onCommit,
  onDragStateChange,
}: {
  label?: string;
  ariaLabel?: string;
  className?: string;
  min: number;
  max: number;
  value: number;
  suffix: string;
  onPreview?: (value: number) => void;
  onCommit: (value: number) => void;
  onDragStateChange?: (dragging: boolean) => void;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const outputRef = useRef<HTMLElement | null>(null);
  const valueRef = useRef(value);
  const draggingRef = useRef(false);
  const rangeLabel = label ?? ariaLabel ?? "滑块";
  const classNames = className ? `slider-field ${className}` : "slider-field";
  const commitKeys = new Set([
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "Home",
    "End",
    "PageUp",
    "PageDown",
  ]);

  function format(next: number) {
    return `${Math.round(next)}${suffix}`;
  }

  function readValue() {
    return Math.round(clamp(inputRef.current?.valueAsNumber ?? valueRef.current, min, max));
  }

  function previewValue() {
    const next = readValue();
    if (outputRef.current) {
      outputRef.current.textContent = format(next);
    }
    onPreview?.(next);
  }

  function commitValue() {
    const next = readValue();
    const previous = valueRef.current;
    valueRef.current = next;
    if (inputRef.current) {
      inputRef.current.value = String(next);
    }
    if (outputRef.current) {
      outputRef.current.textContent = format(next);
    }
    if (next !== previous) {
      onCommit(next);
    }
  }

  function setDragging(next: boolean) {
    if (draggingRef.current === next) return;
    draggingRef.current = next;
    onDragStateChange?.(next);
  }

  function endInteraction() {
    commitValue();
    setDragging(false);
  }

  function handleKeyUp(e: ReactKeyboardEvent<HTMLInputElement>) {
    if (commitKeys.has(e.key)) {
      commitValue();
    }
  }

  useEffect(() => {
    const next = Math.round(clamp(value, min, max));
    valueRef.current = next;
    if (inputRef.current) {
      inputRef.current.value = String(next);
    }
    if (outputRef.current) {
      outputRef.current.textContent = format(next);
    }
  }, [value, min, max, suffix]);

  return (
    <div className={classNames}>
      {label && (
        <label>
          <span>{label}</span>
          <b ref={outputRef}>{format(value)}</b>
        </label>
      )}
      <input
        ref={inputRef}
        type="range"
        min={min}
        max={max}
        defaultValue={value}
        aria-label={rangeLabel}
        onInput={previewValue}
        onPointerDown={(e) => {
          setDragging(true);
          try {
            e.currentTarget.setPointerCapture?.(e.pointerId);
          } catch {
            // Some WebView range controls do not allow pointer capture; drag still works without it.
          }
        }}
        onPointerUp={endInteraction}
        onPointerCancel={endInteraction}
        onLostPointerCapture={() => setDragging(false)}
        onTouchEnd={endInteraction}
        onBlur={endInteraction}
        onKeyUp={handleKeyUp}
      />
      {!label && <b ref={outputRef}>{format(value)}</b>}
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="field">
      <label>{label}</label>
      {children}
      {hint && <span>{hint}</span>}
    </div>
  );
}

function EggLogo({ large = false }: { large?: boolean }) {
  return (
    <svg
      className={`pixel-icon egg-logo ${large ? "large" : ""}`}
      viewBox="0 0 32 32"
      aria-hidden="true"
    >
      <rect x="8" y="6" width="16" height="4" />
      <rect x="4" y="10" width="24" height="12" />
      <rect x="8" y="22" width="16" height="4" />
      <rect className="cut" x="8" y="12" width="4" height="4" />
      <rect className="cut" x="20" y="12" width="4" height="4" />
      <rect className="cut" x="14" y="18" width="4" height="4" />
      <rect className="accent" x="14" y="2" width="4" height="4" />
      <rect className="accent" x="2" y="14" width="4" height="4" />
      <rect className="accent" x="26" y="14" width="4" height="4" />
    </svg>
  );
}

function IconPlus() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="7" y="3" width="2" height="10" />
      <rect x="3" y="7" width="10" height="2" />
    </svg>
  );
}

function IconSend() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="3" width="10" height="2" />
      <rect x="5" y="5" width="8" height="2" />
      <rect x="7" y="7" width="6" height="2" />
      <rect x="5" y="9" width="8" height="2" />
      <rect x="3" y="11" width="10" height="2" />
    </svg>
  );
}

function IconGear() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="6" y="1" width="4" height="3" />
      <rect x="6" y="12" width="4" height="3" />
      <rect x="1" y="6" width="3" height="4" />
      <rect x="12" y="6" width="3" height="4" />
      <rect x="5" y="5" width="6" height="6" />
      <rect className="cut" x="7" y="7" width="2" height="2" />
    </svg>
  );
}

function IconCheck() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="8" width="2" height="2" />
      <rect x="5" y="10" width="2" height="2" />
      <rect x="7" y="8" width="2" height="2" />
      <rect x="9" y="6" width="2" height="2" />
      <rect x="11" y="4" width="2" height="2" />
    </svg>
  );
}

function IconCross() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="3" width="2" height="2" />
      <rect x="5" y="5" width="2" height="2" />
      <rect x="7" y="7" width="2" height="2" />
      <rect x="9" y="9" width="2" height="2" />
      <rect x="11" y="11" width="2" height="2" />
      <rect x="11" y="3" width="2" height="2" />
      <rect x="9" y="5" width="2" height="2" />
      <rect x="5" y="9" width="2" height="2" />
      <rect x="3" y="11" width="2" height="2" />
    </svg>
  );
}
