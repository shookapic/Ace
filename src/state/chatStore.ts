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

function loadLocalConversations(): LocalConversation[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as LocalConversation[]) : [];
  } catch {
    return [];
  }
}

function saveLocalConversations(list: LocalConversation[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(list));
  } catch {
    /* quota exceeded or storage disabled — non-fatal */
  }
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
  setProvider: (provider: ProviderId) => void;
  setModel: (provider: ProviderId, modelId: string) => void;
  fetchModels: (provider: ProviderId) => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  pickAttachments: () => Promise<void>;
  removeAttachment: (index: number) => void;
  fetchConversations: (provider: ProviderId) => Promise<void>;
  loadConversation: (provider: ProviderId, conversationId: string, title?: string) => Promise<void>;
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
const pendingRequests = new Map<string, string>();

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
  localConversations: loadLocalConversations(),
  activeConversationId: null,
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
    const provider = get().provider;
    const trimmed = text.trim();
    const attachments = get().pendingAttachments;
    if (!provider || (!trimmed && attachments.length === 0) || get().sending) return;

    const userMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: trimmed,
      attachments: attachments.length > 0 ? attachments : undefined,
    };
    const assistantId = crypto.randomUUID();
    const requestId = crypto.randomUUID();
    const conversationId = get().activeConversationId ?? crypto.randomUUID();
    const history = [...get().messages, userMessage].map((m) => ({
      role: m.role,
      content: m.content,
      attachments: m.attachments,
    }));
    const model = get().selectedModel[provider] ?? undefined;

    pendingRequests.set(requestId, assistantId);
    set((s) => ({
      messages: [...s.messages, userMessage, { id: assistantId, role: "assistant", content: "" }],
      sending: true,
      error: null,
      pendingAttachments: [],
      activeConversationId: conversationId,
    }));

    try {
      await invoke("send_chat_message", { provider, requestId, model, messages: history });
    } catch (err) {
      pendingRequests.delete(requestId);
      set({ sending: false, error: String(err) });
    }
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
    }),
  connectClaudeWeb: async () => {
    try {
      await invoke("open_claude_login");
    } catch (err) {
      set({ conversationsError: String(err) });
    }
  },
}));

// When the embedded claude.ai login captures a session, refresh the history list.
listen("claude-session://ready", () => {
  const { provider, fetchConversations } = useChatStore.getState();
  if (provider === "anthropic") fetchConversations("anthropic");
});

listen<ChunkEvent>("chat://chunk", (event) => {
  const { requestId, delta } = event.payload;
  const assistantId = pendingRequests.get(requestId);
  if (!assistantId) return;
  useChatStore.setState((s) => ({
    messages: s.messages.map((m) =>
      m.id === assistantId ? { ...m, content: m.content + delta } : m
    ),
  }));
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
  pendingRequests.delete(event.payload.requestId);
  useChatStore.setState({ sending: false });
  persistActiveConversation();
});

listen<ChatErrorEvent>("chat://error", (event) => {
  const assistantId = pendingRequests.get(event.payload.requestId);
  pendingRequests.delete(event.payload.requestId);
  useChatStore.setState((s) => ({
    sending: false,
    error: event.payload.error,
    // Drop the empty assistant placeholder bubble — nothing streamed into it before the error.
    messages:
      assistantId && s.messages.find((m) => m.id === assistantId)?.content === ""
        ? s.messages.filter((m) => m.id !== assistantId)
        : s.messages,
  }));
});
