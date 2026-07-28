import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useState } from "react";
import { AnthropicLogo, OpenAILogo } from "../components/branding/ProviderLogos";
import { SettingsIcon } from "../components/ui/Icons";
import { useAuthStore, type ProviderId } from "../state/authStore";
import { useUiStore } from "../state/uiStore";

const PROVIDERS = [
  { id: "anthropic", label: "Anthropic", Logo: AnthropicLogo },
  { id: "openai", label: "OpenAI", Logo: OpenAILogo },
] as const;

export function Login() {
  const [activeProvider, setActiveProvider] = useState<ProviderId | null>(null);
  const [message, setMessage] = useState("Connect a provider to start");
  const [error, setError] = useState<string | null>(null);
  const { justConnected, startLogin } = useAuthStore();
  const toggleSettings = useUiStore((s) => s.toggleSettings);

  useEffect(() => {
    if (!justConnected) return;
    const timeout = setTimeout(() => {
      useAuthStore.setState((s) =>
        s.justConnected?.key === justConnected.key ? { justConnected: null } : s
      );
    }, 2400);
    return () => clearTimeout(timeout);
  }, [justConnected]);

  async function connectProvider(provider: ProviderId) {
    setActiveProvider(provider);
    setError(null);
    setMessage(
      provider === "anthropic"
        ? "Opening Claude Code's sign-in in your browser..."
        : "Opening OpenAI sign-in in your browser..."
    );

    try {
      await startLogin(provider);
      setMessage("Finish signing in — Ace will pick it up automatically.");
    } catch (authError) {
      setError(String(authError));
      setMessage("Sign-in could not be started");
    } finally {
      setActiveProvider(null);
    }
  }

  return (
    <motion.div
      className="relative flex h-full w-full flex-col items-center justify-center gap-4 overflow-y-auto px-6 py-8"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.4 }}
    >
      <button
        type="button"
        onClick={toggleSettings}
        aria-label="Settings"
        className="absolute right-2.5 top-2.5 flex h-8 w-8 items-center justify-center rounded-lg text-white/40 transition hover:bg-white/[0.06] hover:text-white/80"
      >
        <SettingsIcon className="h-[18px] w-[18px]" />
      </button>

      <AnimatePresence mode="wait">
        {justConnected ? (
          <motion.p
            key={justConnected.key}
            initial={{ opacity: 0, y: 6, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -6, scale: 0.96 }}
            transition={{ type: "spring", stiffness: 300, damping: 20 }}
            className="text-sm font-medium text-emerald-300"
          >
            Logged in with {providerLabel(justConnected.provider)} !
          </motion.p>
        ) : (
          <motion.p
            key="status-message"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="text-sm text-white/70"
          >
            {message}
          </motion.p>
        )}
      </AnimatePresence>
      <div className="flex items-center gap-8">
        {PROVIDERS.map(({ id, label, Logo }) => (
          <div key={id} className="flex flex-col items-center gap-2">
            <motion.button
              type="button"
              aria-label={`Connect ${label}`}
              disabled={activeProvider !== null}
              onClick={() => connectProvider(id)}
              className="text-white/90 disabled:cursor-wait disabled:text-white/30"
              whileHover={{ scale: activeProvider ? 1 : 1.2 }}
              whileTap={{ scale: activeProvider ? 1 : 1.05 }}
              transition={{ type: "spring", stiffness: 400, damping: 15 }}
            >
              <Logo className="h-8 w-8" />
            </motion.button>
          </div>
        ))}
      </div>
      {error ? (
        <p className="max-w-xs text-center text-[10px] leading-relaxed text-red-200/70">
          {error}
        </p>
      ) : null}
      <p className="max-w-xs text-center text-[10px] leading-relaxed text-white/30">
        Ace signs you in directly with your OpenAI or Anthropic account. Nothing is
        shared with third parties.
      </p>
    </motion.div>
  );
}

function providerLabel(provider: ProviderId) {
  return provider === "openai" ? "OpenAI" : "Anthropic";
}
