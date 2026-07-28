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
  setProvider: (provider: ProviderId) => void;
  setModel: (provider: ProviderId, modelId: string) => void;
  fetchModels: (provider: ProviderId) => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  pickAttachments: () => Promise<void>;
  removeAttachment: (index: number) => void;
  fetchConversations: (provider: ProviderId) => Promise<void>;
  loadConversation: (provider: ProviderId, conversationId: string, title?: string) => Promise<void>;
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
  setProvider: (provider) => set({ provider }),
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
        error: null,
      });
    } catch (err) {
      set({ conversationsError: String(err) });
    }
  },
  newConversation: () =>
    set({ messages: [], activeTitle: null, error: null, pendingAttachments: [] }),
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

listen<DoneEvent>("chat://done", (event) => {
  pendingRequests.delete(event.payload.requestId);
  useChatStore.setState({ sending: false });
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
