import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ProviderId } from "./authStore";

export interface Attachment {
  name: string;
  mime: string;
  dataBase64: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  attachments?: Attachment[];
}

interface ChunkEvent {
  requestId: string;
  delta: string;
}

interface DoneEvent {
  requestId: string;
}

interface ChatErrorEvent {
  requestId: string;
  error: string;
}

export interface ModelInfo {
  id: string;
  label: string;
}

export interface ConversationSummary {
  id: string;
  title: string;
  updatedAt: string | null;
}

/** A chat saved on this device (survives restarts, unlike the live session). */
export interface LocalConversation {
  id: string;
  provider: ProviderId;
  title: string;
  updatedAt: string;
  messages: ChatMessage[];
}

const STORAGE_KEY = "ace.conversations.v1";

// Legacy plaintext store — read only, for one-time migration into the encrypted
// Rust-backed store.
function readLegacyLocalConversations(): LocalConversation[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as LocalConversation[]) : [];
  } catch {
    return [];
  }
}

// Persist through the Rust side, which AES-encrypts the blob to disk. Fire and
// forget — history saving must never block or throw into the UI.
function saveLocalConversations(list: LocalConversation[]) {
  invoke("save_conversations", { json: JSON.stringify(list) }).catch((err) =>
    console.error("save_conversations failed", err)
  );
}

function deriveTitle(messages: ChatMessage[]): string {
  const firstUser = messages.find((m) => m.role === "user");
  const text = firstUser?.content.trim() ?? "";
  if (!text) return "New conversation";
  return text.length > 48 ? `${text.slice(0, 48)}…` : text;
}

interface ChatState {
  provider: ProviderId | null;
  messages: ChatMessage[];
  sending: boolean;
  error: string | null;
  models: Record<ProviderId, ModelInfo[]>;
  selectedModel: Record<ProviderId, string | null>;
  modelsLoading: boolean;
  modelsError: string | null;
  pendingAttachments: Attachment[];
  attachmentsError: string | null;
  conversations: ConversationSummary[];
  conversationsLoading: boolean;
  conversationsError: string | null;
  activeTitle: string | null;
  localConversations: LocalConversation[];
  activeConversationId: string | null;
  lastConversationByProvider: Record<ProviderId, string | null>;
  activeRequestId: string | null;
  compareMode: boolean;
  compareThreads: Record<ProviderId, ChatMessage[]>;
  setProvider: (provider: ProviderId) => void;
  setModel: (provider: ProviderId, modelId: string) => void;
  fetchModels: (provider: ProviderId) => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  toggleCompareMode: () => void;
  stopStreaming: () => void;
  regenerateLast: () => Promise<void>;
  editAndResend: (messageId: string, newText: string) => Promise<void>;
  pickAttachments: () => Promise<void>;
  captureScreenshot: () => Promise<void>;
  removeAttachment: (index: number) => void;
  fetchConversations: (provider: ProviderId) => Promise<void>;
  loadConversation: (provider: ProviderId, conversationId: string, title?: string) => Promise<void>;
  hydrateConversations: () => Promise<void>;
  loadLocalConversation: (id: string) => void;
  deleteLocalConversation: (id: string) => void;
  deleteLocalConversations: (ids: string[]) => void;
  clearLocalConversations: () => void;
  newConversation: () => void;
  connectClaudeWeb: () => Promise<void>;
}

// Maps an in-flight requestId to the assistant message it should append streamed
// text into — event listeners are module-scoped (outside the store), so this is
// how they find the right message without threading state through every event.
// Which thread a streamed response writes into: the single main chat, or one
// provider's column in compare mode.
type PendingTarget = "main" | ProviderId;
const pendingRequests = new Map<string, { assistantId: string; thread: PendingTarget }>();

function toHistory(msgs: ChatMessage[]) {
  return msgs.map((m) => ({ role: m.role, content: m.content, attachments: m.attachments }));
}

// Shared send path for the main chat: takes the full thread (already ending at
// the user turn to answer), appends an assistant placeholder, and kicks off
// streaming. Used by sendMessage, regenerateLast, and editAndResend.
async function runCompletion(baseMessages: ChatMessage[]) {
  const state = useChatStore.getState();
  const provider = state.provider;
  if (!provider) return;

  const assistantId = crypto.randomUUID();
  const requestId = crypto.randomUUID();
  const conversationId = state.activeConversationId ?? crypto.randomUUID();
  const model = state.selectedModel[provider] ?? undefined;

  pendingRequests.set(requestId, { assistantId, thread: "main" });
  useChatStore.setState({
    messages: [...baseMessages, { id: assistantId, role: "assistant", content: "" }],
    sending: true,
    error: null,
    pendingAttachments: [],
    activeConversationId: conversationId,
    activeRequestId: requestId,
  });

  try {
    await invoke("send_chat_message", {
      provider,
      requestId,
      model,
      messages: toHistory(baseMessages),
    });
  } catch (err) {
    pendingRequests.delete(requestId);
    useChatStore.setState({ sending: false, error: String(err), activeRequestId: null });
  }
}

// Compare mode: fan the same user turn out to both providers, each streaming
// into its own column.
async function sendCompare(userMessage: ChatMessage) {
  const state = useChatStore.getState();
  const providers: ProviderId[] = ["anthropic", "openai"];

  const newThreads = { ...state.compareThreads };
  const requests: { provider: ProviderId; requestId: string; history: ReturnType<typeof toHistory> }[] = [];

  for (const provider of providers) {
    const assistantId = crypto.randomUUID();
    const requestId = crypto.randomUUID();
    const base = [...state.compareThreads[provider], userMessage];
    newThreads[provider] = [...base, { id: assistantId, role: "assistant", content: "" }];
    pendingRequests.set(requestId, { assistantId, thread: provider });
    requests.push({ provider, requestId, history: toHistory(base) });
  }

  useChatStore.setState({
    compareThreads: newThreads,
    sending: true,
    error: null,
    pendingAttachments: [],
  });

  for (const r of requests) {
    const model = state.selectedModel[r.provider] ?? undefined;
    invoke("send_chat_message", {
      provider: r.provider,
      requestId: r.requestId,
      model,
      messages: r.history,
    }).catch((err) => {
      pendingRequests.delete(r.requestId);
      useChatStore.setState({
        error: String(err),
        sending: pendingRequests.size > 0,
      });
    });
  }
}

export const useChatStore = create<ChatState>((set, get) => ({
  provider: null,
  messages: [],
  sending: false,
  error: null,
  models: { anthropic: [], openai: [] },
  selectedModel: { anthropic: null, openai: null },
  modelsLoading: false,
  modelsError: null,
  pendingAttachments: [],
  attachmentsError: null,
  conversations: [],
  conversationsLoading: false,
  conversationsError: null,
  activeTitle: null,
  localConversations: [],
  activeConversationId: null,
  activeRequestId: null,
  compareMode: false,
  compareThreads: { anthropic: [], openai: [] },
  lastConversationByProvider: { anthropic: null, openai: null },
  // Switching providers parks the current chat and restores the one you last had
  // open for the provider you're switching to — so bouncing between Claude and
  // ChatGPT keeps each side's thread instead of merging them.
  setProvider: (provider) =>
    set((s) => {
      if (s.provider === provider) return { provider };
      const remembered = { ...s.lastConversationByProvider };
      if (s.provider) remembered[s.provider] = s.activeConversationId;
      const targetId = remembered[provider];
      const conv = targetId ? s.localConversations.find((c) => c.id === targetId) : undefined;
      return {
        provider,
        lastConversationByProvider: remembered,
        messages: conv ? conv.messages.map((m) => ({ ...m })) : [],
        activeConversationId: conv ? conv.id : null,
        activeTitle: conv ? conv.title : null,
        error: null,
        pendingAttachments: [],
      };
    }),
  setModel: (provider, modelId) =>
    set((s) => ({ selectedModel: { ...s.selectedModel, [provider]: modelId } })),
  fetchModels: async (provider) => {
    set({ modelsLoading: true, modelsError: null });
    try {
      const models = await invoke<ModelInfo[]>("list_models", { provider });
      set((s) => ({
        models: { ...s.models, [provider]: models },
        selectedModel: {
          ...s.selectedModel,
          [provider]: s.selectedModel[provider] ?? models[0]?.id ?? null,
        },
      }));
    } catch (err) {
      console.error("list_models failed", err);
      set({ modelsError: String(err) });
    } finally {
      set({ modelsLoading: false });
    }
  },
  sendMessage: async (text) => {
    const state = get();
    const trimmed = text.trim();
    const attachments = state.pendingAttachments;
    if ((!trimmed && attachments.length === 0) || state.sending) return;

    const userMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: trimmed,
      attachments: attachments.length > 0 ? attachments : undefined,
    };

    if (state.compareMode) {
      await sendCompare(userMessage);
      return;
    }
    if (!state.provider) return;
    await runCompletion([...state.messages, userMessage]);
  },
  toggleCompareMode: () => {
    const s = get();
    if (s.sending) return;
    const next = !s.compareMode;
    // Seed each column from the current single-chat thread when entering compare.
    set({
      compareMode: next,
      compareThreads: next
        ? {
            anthropic: s.messages.map((m) => ({ ...m })),
            openai: s.messages.map((m) => ({ ...m })),
          }
        : s.compareThreads,
    });
  },
  stopStreaming: () => {
    // Cancels every in-flight request — one in the main chat, or both columns
    // in compare mode.
    for (const requestId of pendingRequests.keys()) {
      invoke("cancel_chat_message", { requestId }).catch(() => {});
    }
  },
  regenerateLast: async () => {
    if (get().sending) return;
    const msgs = get().messages;
    const lastUserIdx = msgs.map((m) => m.role).lastIndexOf("user");
    if (lastUserIdx === -1) return;
    // Re-run from the thread up to and including the last user turn, dropping the
    // assistant answer that followed it.
    await runCompletion(msgs.slice(0, lastUserIdx + 1));
  },
  editAndResend: async (messageId, newText) => {
    if (get().sending) return;
    const msgs = get().messages;
    const idx = msgs.findIndex((m) => m.id === messageId);
    if (idx === -1 || msgs[idx].role !== "user") return;
    const edited: ChatMessage = { ...msgs[idx], content: newText.trim() };
    await runCompletion([...msgs.slice(0, idx), edited]);
  },
  pickAttachments: async () => {
    try {
      const picked = await invoke<Attachment[]>("pick_files");
      if (picked.length > 0) {
        set((s) => ({ pendingAttachments: [...s.pendingAttachments, ...picked], attachmentsError: null }));
      }
    } catch (err) {
      set({ attachmentsError: String(err) });
    }
  },
  captureScreenshot: async () => {
    try {
      const shot = await invoke<Attachment>("capture_screen");
      set((s) => ({ pendingAttachments: [...s.pendingAttachments, shot], attachmentsError: null }));
    } catch (err) {
      set({ attachmentsError: String(err) });
    }
  },
  removeAttachment: (index) =>
    set((s) => ({ pendingAttachments: s.pendingAttachments.filter((_, i) => i !== index) })),
  fetchConversations: async (provider) => {
    set({ conversationsLoading: true, conversationsError: null });
    try {
      const conversations = await invoke<ConversationSummary[]>("list_conversations", { provider });
      set({ conversations });
    } catch (err) {
      set({ conversationsError: String(err), conversations: [] });
    } finally {
      set({ conversationsLoading: false });
    }
  },
  loadConversation: async (provider, conversationId, title) => {
    set({ conversationsError: null });
    try {
      const history = await invoke<{ role: "user" | "assistant"; content: string }[]>(
        "get_conversation",
        { provider, conversationId }
      );
      set({
        messages: history.map((m) => ({ id: crypto.randomUUID(), role: m.role, content: m.content })),
        activeTitle: title ?? null,
        // Give it a local id so replying continues it and saves it on this device.
        activeConversationId: crypto.randomUUID(),
        error: null,
      });
    } catch (err) {
      set({ conversationsError: String(err) });
    }
  },
  hydrateConversations: async () => {
    try {
      const json = await invoke<string>("load_conversations");
      let list = JSON.parse(json) as LocalConversation[];
      // One-time migration from the old plaintext localStorage store.
      if (list.length === 0) {
        const legacy = readLegacyLocalConversations();
        if (legacy.length > 0) {
          list = legacy;
          saveLocalConversations(list);
          try {
            localStorage.removeItem(STORAGE_KEY);
          } catch {
            /* ignore */
          }
        }
      }
      set({ localConversations: list });
    } catch (err) {
      console.error("hydrateConversations failed", err);
    }
  },
  loadLocalConversation: (id) => {
    const conv = get().localConversations.find((c) => c.id === id);
    if (!conv) return;
    set({
      provider: conv.provider,
      messages: conv.messages.map((m) => ({ ...m })),
      activeConversationId: conv.id,
      activeTitle: conv.title,
      error: null,
      pendingAttachments: [],
    });
  },
  deleteLocalConversation: (id) => get().deleteLocalConversations([id]),
  deleteLocalConversations: (ids) => {
    const remove = new Set(ids);
    const next = get().localConversations.filter((c) => !remove.has(c.id));
    saveLocalConversations(next);
    set((s) => {
      const remembered = { ...s.lastConversationByProvider };
      (Object.keys(remembered) as ProviderId[]).forEach((p) => {
        if (remembered[p] && remove.has(remembered[p]!)) remembered[p] = null;
      });
      return {
        localConversations: next,
        lastConversationByProvider: remembered,
        activeConversationId:
          s.activeConversationId && remove.has(s.activeConversationId) ? null : s.activeConversationId,
      };
    });
  },
  clearLocalConversations: () => {
    saveLocalConversations([]);
    set({
      localConversations: [],
      lastConversationByProvider: { anthropic: null, openai: null },
      activeConversationId: null,
    });
  },
  newConversation: () =>
    set({
      messages: [],
      activeTitle: null,
      activeConversationId: null,
      error: null,
      pendingAttachments: [],
      compareThreads: { anthropic: [], openai: [] },
    }),
  connectClaudeWeb: async () => {
    try {
      await invoke("open_claude_login");
    } catch (err) {
      set({ conversationsError: String(err) });
    }
  },
}));

// Ctrl+Shift+S (fired from Rust) grabs a screenshot into the composer.
listen("shortcut://screenshot", () => {
  useChatStore.getState().captureScreenshot();
});

// When the embedded claude.ai login captures a session, refresh the history list.
listen("claude-session://ready", () => {
  const { provider, fetchConversations } = useChatStore.getState();
  if (provider === "anthropic") fetchConversations("anthropic");
});

listen<ChunkEvent>("chat://chunk", (event) => {
  const { requestId, delta } = event.payload;
  const entry = pendingRequests.get(requestId);
  if (!entry) return;
  useChatStore.setState((s) => {
    const append = (msgs: ChatMessage[]) =>
      msgs.map((m) => (m.id === entry.assistantId ? { ...m, content: m.content + delta } : m));
    if (entry.thread === "main") return { messages: append(s.messages) };
    const provider = entry.thread;
    return { compareThreads: { ...s.compareThreads, [provider]: append(s.compareThreads[provider]) } };
  });
});

// Upsert the live conversation into on-device history so it survives restarts.
function persistActiveConversation() {
  const { provider, messages, activeConversationId, activeTitle, localConversations } =
    useChatStore.getState();
  if (!provider || !activeConversationId || messages.length === 0) return;
  const title = activeTitle ?? deriveTitle(messages);
  // Drop base64 attachment bytes before saving — they'd blow the localStorage
  // quota fast. The live session keeps them; reopened chats show a file chip.
  const lean = messages.map((m) =>
    m.attachments && m.attachments.length > 0
      ? { ...m, attachments: m.attachments.map((a) => ({ name: a.name, mime: a.mime, dataBase64: "" })) }
      : m
  );
  const entry: LocalConversation = {
    id: activeConversationId,
    provider,
    title,
    updatedAt: new Date().toISOString(),
    messages: lean,
  };
  const next = [entry, ...localConversations.filter((c) => c.id !== activeConversationId)];
  saveLocalConversations(next);
  useChatStore.setState({ localConversations: next, activeTitle: title });
}

listen<DoneEvent>("chat://done", (event) => {
  const entry = pendingRequests.get(event.payload.requestId);
  pendingRequests.delete(event.payload.requestId);
  if (entry?.thread === "main") persistActiveConversation();
  if (pendingRequests.size === 0) {
    useChatStore.setState({ sending: false, activeRequestId: null });
  }
});

listen<ChatErrorEvent>("chat://error", (event) => {
  const entry = pendingRequests.get(event.payload.requestId);
  pendingRequests.delete(event.payload.requestId);
  const stillRunning = pendingRequests.size > 0;
  useChatStore.setState((s) => {
    const base: Partial<ChatState> = {
      sending: stillRunning,
      error: event.payload.error,
      activeRequestId: stillRunning ? s.activeRequestId : null,
    };
    if (!entry) return base;
    // Drop the empty placeholder bubble — nothing streamed in before the error.
    if (entry.thread === "main") {
      const drop = s.messages.find((m) => m.id === entry.assistantId)?.content === "";
      return { ...base, messages: drop ? s.messages.filter((m) => m.id !== entry.assistantId) : s.messages };
    }
    const provider = entry.thread;
    const thread = s.compareThreads[provider];
    const drop = thread.find((m) => m.id === entry.assistantId)?.content === "";
    return {
      ...base,
      compareThreads: drop
        ? { ...s.compareThreads, [provider]: thread.filter((m) => m.id !== entry.assistantId) }
        : s.compareThreads,
    };
  });
});
