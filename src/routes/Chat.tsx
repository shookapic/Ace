import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnthropicLogo, OpenAILogo } from "../components/branding/ProviderLogos";
import {
  CheckIcon,
  ChevronDownIcon,
  CloseIcon,
  HistoryIcon,
  MicIcon,
  PaperclipIcon,
  PlusIcon,
  SendIcon,
  SettingsIcon,
  SpinnerIcon,
  StopIcon,
} from "../components/ui/Icons";
import { CopyButton, MarkdownMessage } from "../components/ui/MarkdownMessage";
import { useAuthStore, type ProviderId } from "../state/authStore";
import { useChatStore, type ModelInfo } from "../state/chatStore";
import { useUiStore } from "../state/uiStore";

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}

const LOGOS: Record<ProviderId, (props: { className?: string }) => React.ReactElement> = {
  anthropic: AnthropicLogo,
  openai: OpenAILogo,
};
const PROVIDER_NAME: Record<ProviderId, string> = { anthropic: "Claude", openai: "OpenAI" };

/** Small ghost icon button used across the header and composer. */
function IconButton({
  onClick,
  label,
  disabled,
  active,
  children,
}: {
  onClick: () => void;
  label: string;
  disabled?: boolean;
  active?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={`flex h-8 w-8 items-center justify-center rounded-lg transition disabled:cursor-not-allowed disabled:opacity-30 ${
        active
          ? "bg-[var(--accent-soft)] text-[color:var(--accent)]"
          : "text-white/50 hover:bg-white/[0.06] hover:text-white"
      }`}
    >
      {children}
    </button>
  );
}

/** Custom model dropdown — native <select> option lists can't be themed, so we
 * render our own popover that opens upward out of the composer. */
function ModelPicker({
  models,
  selectedId,
  onSelect,
}: {
  models: ModelInfo[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const selected = models.find((m) => m.id === selectedId);

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex max-w-[170px] items-center gap-1 rounded-md px-1.5 py-1 text-[11px] text-white/45 transition hover:text-white/80"
      >
        <span className="truncate">{selected?.label ?? "Model"}</span>
        <ChevronDownIcon className="h-3 w-3 shrink-0" />
      </button>

      <AnimatePresence>
        {open ? (
          <>
            <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
            <motion.div
              initial={{ opacity: 0, y: 6, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: 6, scale: 0.98 }}
              transition={{ type: "spring", stiffness: 460, damping: 32 }}
              className="absolute bottom-full left-0 z-50 mb-1.5 max-h-60 w-52 overflow-y-auto rounded-xl border border-white/10 bg-neutral-950/95 p-1 shadow-2xl backdrop-blur-xl"
            >
              {models.map((m) => {
                const active = m.id === selectedId;
                return (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => {
                      onSelect(m.id);
                      setOpen(false);
                    }}
                    className={`flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs transition ${
                      active
                        ? "bg-[var(--accent-soft)] text-[color:var(--accent)]"
                        : "text-white/70 hover:bg-white/[0.07] hover:text-white"
                    }`}
                  >
                    <span className="truncate">{m.label}</span>
                    {active ? <CheckIcon className="h-3.5 w-3.5 shrink-0" /> : null}
                  </button>
                );
              })}
            </motion.div>
          </>
        ) : null}
      </AnimatePresence>
    </div>
  );
}

export function Chat() {
  const providers = useAuthStore((s) => s.providers);
  const startLogin = useAuthStore((s) => s.startLogin);
  const {
    provider,
    messages,
    sending,
    error,
    models,
    selectedModel,
    modelsLoading,
    modelsError,
    pendingAttachments,
    attachmentsError,
    conversations,
    conversationsLoading,
    conversationsError,
    activeTitle,
    setProvider,
    setModel,
    fetchModels,
    sendMessage,
    pickAttachments,
    removeAttachment,
    fetchConversations,
    loadConversation,
    localConversations,
    loadLocalConversation,
    deleteLocalConversation,
    newConversation,
    connectClaudeWeb,
  } = useChatStore();
  const [input, setInput] = useState("");
  const [historyQuery, setHistoryQuery] = useState("");
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [voiceError, setVoiceError] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const toggleSettings = useUiStore((s) => s.toggleSettings);
  const justConnected = useAuthStore((s) => s.justConnected);
  const openaiConnected = providers.openai.available;

  const connected = (Object.keys(providers) as ProviderId[]).filter((id) => providers[id].available);
  const disconnected = (Object.keys(providers) as ProviderId[]).filter(
    (id) => !providers[id].available
  );

  useEffect(() => {
    if (!provider && connected.length > 0) setProvider(connected[0]);
  }, [provider, connected, setProvider]);

  // Signing into a provider makes it the active one — the provider you just
  // connected is the one you want to talk to.
  useEffect(() => {
    if (justConnected && providers[justConnected.provider].available) {
      setProvider(justConnected.provider);
    }
  }, [justConnected, providers, setProvider]);

  useEffect(() => {
    if (provider && models[provider].length === 0 && !modelsLoading) fetchModels(provider);
  }, [provider, models, modelsLoading, fetchModels]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages]);

  async function handleSend() {
    const text = input;
    setInput("");
    await sendMessage(text);
  }

  async function toggleRecording() {
    if (recording) {
      mediaRecorderRef.current?.stop();
      return;
    }
    setVoiceError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType = MediaRecorder.isTypeSupported("audio/webm") ? "audio/webm" : "audio/ogg";
      const recorder = new MediaRecorder(stream, { mimeType });
      audioChunksRef.current = [];
      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) audioChunksRef.current.push(e.data);
      };
      recorder.onstop = async () => {
        stream.getTracks().forEach((t) => t.stop());
        setRecording(false);
        setTranscribing(true);
        try {
          const blob = new Blob(audioChunksRef.current, { type: mimeType });
          const audioBase64 = arrayBufferToBase64(await blob.arrayBuffer());
          const text = await invoke<string>("transcribe_audio", { audioBase64, mime: mimeType });
          setInput((prev) => (prev ? `${prev} ${text}`.trim() : text));
        } catch (err) {
          setVoiceError(String(err));
        } finally {
          setTranscribing(false);
        }
      };
      mediaRecorderRef.current = recorder;
      recorder.start();
      setRecording(true);
    } catch (err) {
      setVoiceError(String(err));
    }
  }

  const canSend = !sending && (input.trim().length > 0 || pendingAttachments.length > 0);

  return (
    <motion.div
      className="flex h-full w-full min-w-0 flex-col"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.35 }}
    >
      {/* Header: provider identity · title · actions */}
      <header className="relative flex h-11 shrink-0 items-center justify-between px-2.5">
        <div className="flex items-center gap-0.5">
          {connected.map((id) => {
            const Logo = LOGOS[id];
            const isActive = provider === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => setProvider(id)}
                aria-label={`Talk to ${PROVIDER_NAME[id]}`}
                title={PROVIDER_NAME[id]}
                className={`flex h-8 items-center gap-1.5 rounded-lg px-2 text-xs font-medium transition ${
                  isActive
                    ? "bg-[var(--accent-soft)] text-[color:var(--accent)]"
                    : "text-white/40 hover:text-white/70"
                }`}
              >
                <Logo className="h-4 w-4" />
                {isActive ? <span>{PROVIDER_NAME[id]}</span> : null}
              </button>
            );
          })}
          {disconnected.map((id) => {
            const Logo = LOGOS[id];
            return (
              <button
                key={id}
                type="button"
                onClick={() => startLogin(id)}
                aria-label={`Connect ${PROVIDER_NAME[id]}`}
                title={`Connect ${PROVIDER_NAME[id]}`}
                className="flex h-8 w-8 items-center justify-center rounded-lg text-white/15 transition hover:text-white/45"
              >
                <Logo className="h-4 w-4" />
              </button>
            );
          })}
        </div>

        {activeTitle ? (
          <span className="pointer-events-none absolute left-1/2 max-w-[45%] -translate-x-1/2 truncate text-[11px] text-white/40">
            {activeTitle}
          </span>
        ) : null}

        <div className="flex items-center gap-0.5">
          <IconButton onClick={newConversation} label="New chat">
            <PlusIcon className="h-[18px] w-[18px]" />
          </IconButton>
          <IconButton
            onClick={() => {
              const next = !historyOpen;
              setHistoryOpen(next);
              if (next && provider) fetchConversations(provider);
            }}
            label="Conversation history"
            active={historyOpen}
          >
            <HistoryIcon className="h-[18px] w-[18px]" />
          </IconButton>
          <IconButton onClick={toggleSettings} label="Settings">
            <SettingsIcon className="h-[18px] w-[18px]" />
          </IconButton>
        </div>

        {/* History dropdown */}
        <AnimatePresence>
          {historyOpen ? (
            <>
              <div className="fixed inset-0 z-30" onClick={() => setHistoryOpen(false)} />
              <motion.div
                initial={{ opacity: 0, y: -6, scale: 0.98 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -6, scale: 0.98 }}
                transition={{ type: "spring", stiffness: 420, damping: 30 }}
                className="absolute right-2.5 top-full z-40 mt-1 flex max-h-80 w-72 flex-col overflow-hidden rounded-xl border border-white/10 bg-neutral-950/95 p-1.5 shadow-2xl backdrop-blur-xl"
              >
                {(() => {
                  const q = historyQuery.trim().toLowerCase();
                  const localMatches = localConversations.filter(
                    (c) => c.provider === provider && (!q || c.title.toLowerCase().includes(q))
                  );
                  const remoteMatches = conversations.filter(
                    (c) => !q || c.title.toLowerCase().includes(q)
                  );
                  return (
                    <>
                      <div className="mb-1 shrink-0 px-1">
                        <input
                          value={historyQuery}
                          onChange={(e) => setHistoryQuery(e.target.value)}
                          placeholder="Search conversations"
                          className="w-full rounded-lg border border-white/10 bg-white/[0.04] px-2.5 py-1.5 text-xs text-white placeholder:text-white/30 focus:border-[color:var(--accent)] focus:outline-none"
                        />
                      </div>
                      <div className="min-h-0 flex-1 overflow-y-auto">
                        {/* Saved on this device */}
                        <p className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-white/35">
                          On this device
                        </p>
                        {localMatches.length === 0 ? (
                          <p className="px-2 pb-1 text-[11px] text-white/30">
                            {q ? "No matches." : "Saved chats appear here."}
                          </p>
                        ) : (
                          localMatches.map((c) => (
                            <div
                              key={c.id}
                              className="group/item flex items-center rounded-lg transition hover:bg-white/[0.07]"
                            >
                              <button
                                type="button"
                                onClick={() => {
                                  setHistoryOpen(false);
                                  loadLocalConversation(c.id);
                                }}
                                className="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-xs text-white/70 group-hover/item:text-white"
                                title={c.title}
                              >
                                {c.title}
                              </button>
                              <button
                                type="button"
                                onClick={() => deleteLocalConversation(c.id)}
                                aria-label={`Delete ${c.title}`}
                                className="mr-1 flex h-5 w-5 shrink-0 items-center justify-center rounded text-white/25 opacity-0 transition hover:bg-white/10 hover:text-white/80 group-hover/item:opacity-100"
                              >
                                <CloseIcon className="h-3 w-3" />
                              </button>
                            </div>
                          ))
                        )}

                        {/* Remote (claude.ai / ChatGPT) */}
                        <p className="mt-1 px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-white/35">
                          From {provider ? PROVIDER_NAME[provider] : ""}
                        </p>
                        {conversationsLoading ? (
                          <p className="px-2 py-1.5 text-xs text-white/40">Loading…</p>
                        ) : conversationsError ? (
                          <div className="px-2 py-1.5">
                            <p className="text-xs leading-relaxed text-amber-300/80">
                              {conversationsError}
                            </p>
                            {provider === "anthropic" ? (
                              <button
                                type="button"
                                onClick={() => connectClaudeWeb()}
                                className="mt-2 w-full rounded-lg bg-[var(--accent)] px-2 py-1.5 text-xs font-medium text-black transition hover:opacity-90"
                              >
                                Connect claude.ai
                              </button>
                            ) : null}
                          </div>
                        ) : remoteMatches.length === 0 ? (
                          <div className="px-2 py-1.5">
                            <p className="text-xs text-white/40">
                              {q ? "No matches." : "No conversations."}
                            </p>
                            {provider === "anthropic" && !q ? (
                              <button
                                type="button"
                                onClick={() => connectClaudeWeb()}
                                className="mt-2 w-full rounded-lg border border-white/12 px-2 py-1.5 text-xs text-white/70 transition hover:border-white/25 hover:text-white"
                              >
                                Connect claude.ai
                              </button>
                            ) : null}
                          </div>
                        ) : (
                          remoteMatches.map((c) => (
                            <button
                              key={c.id}
                              type="button"
                              onClick={async () => {
                                if (!provider) return;
                                setHistoryOpen(false);
                                await loadConversation(provider, c.id, c.title);
                              }}
                              className="block w-full truncate rounded-lg px-2 py-1.5 text-left text-xs text-white/70 transition hover:bg-white/[0.07] hover:text-white"
                              title={c.title}
                            >
                              {c.title}
                            </button>
                          ))
                        )}
                      </div>
                    </>
                  );
                })()}
              </motion.div>
            </>
          ) : null}
        </AnimatePresence>
      </header>

      {/* Messages */}
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-3.5 py-3">
        {messages.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            {provider ? (
              (() => {
                const Logo = LOGOS[provider];
                return <Logo className="h-8 w-8 text-[color:var(--accent)] opacity-70" />;
              })()
            ) : null}
            <div>
              <p className="text-sm text-white/70">
                {provider ? `Ask ${PROVIDER_NAME[provider]} anything` : "Connect a provider"}
              </p>
              <p className="mt-1 text-xs text-white/30">
                Attach files, dictate, or open past chats from the header.
              </p>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {messages.map((m) =>
              m.role === "user" ? (
                <div
                  key={m.id}
                  className="max-w-[85%] min-w-0 self-end overflow-hidden rounded-2xl rounded-br-md bg-[var(--accent-soft)] px-3.5 py-2 text-sm text-white"
                >
                  {m.attachments && m.attachments.length > 0 ? (
                    <div className="mb-1.5 flex flex-wrap gap-1.5">
                      {m.attachments.map((att, i) =>
                        att.mime.startsWith("image/") && att.dataBase64 ? (
                          <img
                            key={`${att.name}-${i}`}
                            src={`data:${att.mime};base64,${att.dataBase64}`}
                            alt={att.name}
                            title={att.name}
                            className="h-16 w-16 rounded-lg border border-white/15 object-cover"
                          />
                        ) : (
                          <span
                            key={`${att.name}-${i}`}
                            className="flex items-center gap-1.5 rounded-lg bg-black/20 px-2 py-1 text-[11px] text-white/80"
                          >
                            <PaperclipIcon className="h-3 w-3 shrink-0" />
                            <span className="truncate max-w-[140px]">{att.name}</span>
                          </span>
                        )
                      )}
                    </div>
                  ) : null}
                  {m.content ? (
                    <div className="max-w-none overflow-hidden break-words [overflow-wrap:anywhere]">
                      <MarkdownMessage content={m.content} />
                    </div>
                  ) : null}
                </div>
              ) : (
                <div key={m.id} className="group/msg min-w-0 max-w-full self-start px-0.5 text-sm text-white/90">
                  <div className="max-w-none overflow-hidden break-words [overflow-wrap:anywhere]">
                    <MarkdownMessage content={m.content || (sending ? "…" : "")} />
                  </div>
                  {m.content ? (
                    <div className="mt-1 opacity-0 transition group-hover/msg:opacity-100">
                      <CopyButton getText={() => m.content} />
                    </div>
                  ) : null}
                </div>
              )
            )}
          </div>
        )}
      </div>

      {/* Composer */}
      <div className="shrink-0 px-3 pb-3 pt-1">
        {error ? (
          <div className="mb-2 flex items-start gap-2 rounded-lg border border-amber-400/20 bg-amber-400/10 px-2.5 py-1.5">
            <span className="mt-px text-xs text-amber-300">⚠</span>
            <p className="text-xs leading-relaxed text-amber-100/90">{error}</p>
          </div>
        ) : null}

        {pendingAttachments.length > 0 ? (
          <div className="mb-2 flex flex-wrap gap-1.5">
            {pendingAttachments.map((att, i) =>
              att.mime.startsWith("image/") ? (
                <span key={`${att.name}-${i}`} className="group/att relative h-14 w-14">
                  <img
                    src={`data:${att.mime};base64,${att.dataBase64}`}
                    alt={att.name}
                    title={att.name}
                    className="h-14 w-14 rounded-lg border border-white/10 object-cover"
                  />
                  <button
                    type="button"
                    onClick={() => removeAttachment(i)}
                    aria-label={`Remove ${att.name}`}
                    className="absolute -right-1.5 -top-1.5 flex h-5 w-5 items-center justify-center rounded-full border border-white/15 bg-neutral-900 text-white/70 transition hover:text-white"
                  >
                    <CloseIcon className="h-3 w-3" />
                  </button>
                </span>
              ) : (
                <span
                  key={`${att.name}-${i}`}
                  className="flex items-center gap-1.5 rounded-lg bg-white/[0.07] py-1 pl-2 pr-1 text-[11px] text-white/70"
                >
                  <PaperclipIcon className="h-3 w-3 shrink-0 text-white/40" />
                  <span className="truncate max-w-[140px]">{att.name}</span>
                  <button
                    type="button"
                    onClick={() => removeAttachment(i)}
                    aria-label={`Remove ${att.name}`}
                    className="flex h-4 w-4 items-center justify-center rounded text-white/40 hover:bg-white/10 hover:text-white"
                  >
                    <CloseIcon className="h-3 w-3" />
                  </button>
                </span>
              )
            )}
          </div>
        ) : null}
        {attachmentsError ? (
          <p className="mb-1.5 text-[11px] text-amber-300/80">{attachmentsError}</p>
        ) : null}
        {voiceError ? <p className="mb-1.5 text-[11px] text-amber-300/80">{voiceError}</p> : null}

        <div className="rounded-2xl border border-white/10 bg-white/[0.03] transition-colors focus-within:border-[color:var(--accent)]">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                handleSend();
              }
            }}
            rows={1}
            placeholder={recording ? "Listening…" : "Message"}
            className="max-h-32 w-full resize-none bg-transparent px-3.5 pt-3 pb-1 text-sm text-white placeholder:text-white/30 focus:outline-none"
          />
          <div className="flex items-center justify-between px-2 pb-2">
            <div className="flex items-center gap-0.5">
              <IconButton onClick={() => pickAttachments()} label="Attach files">
                <PaperclipIcon className="h-[18px] w-[18px]" />
              </IconButton>
              <IconButton
                onClick={toggleRecording}
                disabled={!openaiConnected || transcribing}
                active={recording}
                label={
                  !openaiConnected
                    ? "Connect OpenAI to dictate"
                    : recording
                      ? "Stop recording"
                      : "Dictate a message"
                }
              >
                {transcribing ? (
                  <SpinnerIcon className="h-[18px] w-[18px] animate-spin" />
                ) : recording ? (
                  <StopIcon className="h-[18px] w-[18px] text-red-400" />
                ) : (
                  <MicIcon className="h-[18px] w-[18px]" />
                )}
              </IconButton>

              {provider && models[provider].length > 0 ? (
                <ModelPicker
                  models={models[provider]}
                  selectedId={selectedModel[provider]}
                  onSelect={(id) => setModel(provider, id)}
                />
              ) : provider && modelsLoading ? (
                <span className="ml-1 text-[11px] text-white/30">Loading models…</span>
              ) : provider && modelsError ? (
                <span className="ml-1 text-[11px] text-amber-300/70" title={modelsError}>
                  Models unavailable
                </span>
              ) : null}
            </div>

            <motion.button
              type="button"
              onClick={handleSend}
              disabled={!canSend}
              aria-label="Send message"
              whileTap={canSend ? { scale: 0.9 } : undefined}
              className="flex h-8 w-8 items-center justify-center rounded-full bg-[var(--accent)] text-black transition disabled:bg-white/[0.08] disabled:text-white/25"
            >
              <SendIcon className="h-[18px] w-[18px]" />
            </motion.button>
          </div>
        </div>
      </div>
    </motion.div>
  );
}
