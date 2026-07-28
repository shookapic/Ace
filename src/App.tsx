import { useEffect, useState } from "react";
import { AnimatePresence } from "framer-motion";
import { TitlebarControls } from "./components/window/TitlebarControls";
import { SettingsPanel } from "./components/settings/SettingsPanel";
import { StartupAnimation } from "./components/branding/StartupAnimation";
import { Login } from "./routes/Login";
import { Chat } from "./routes/Chat";
import { useAuthStore } from "./state/authStore";
import { useChatStore } from "./state/chatStore";
import { useWindowStore } from "./state/windowStore";
import { NEUTRAL_ACCENT, PROVIDER_ACCENT } from "./theme";

function App() {
  const [showStartup, setShowStartup] = useState(true);
  const [checkedAuth, setCheckedAuth] = useState(false);
  const providers = useAuthStore((s) => s.providers);
  const refreshStatus = useAuthStore((s) => s.refreshStatus);
  const activeProvider = useChatStore((s) => s.provider);
  const toggleShortcut = useWindowStore((s) => s.toggleShortcut);
  const applyToggleShortcut = useWindowStore((s) => s.applyToggleShortcut);
  const accent = activeProvider ? PROVIDER_ACCENT[activeProvider] : NEUTRAL_ACCENT;

  useEffect(() => {
    Promise.all([refreshStatus("openai"), refreshStatus("anthropic")]).finally(() =>
      setCheckedAuth(true)
    );
  }, [refreshStatus]);

  useEffect(() => {
    applyToggleShortcut(toggleShortcut).catch((err) =>
      console.error("failed to register toggle shortcut", err)
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const connected = providers.openai.available || providers.anthropic.available;
  const showChat = checkedAuth && connected;

  return (
    <main
      className="flex h-full w-full flex-col overflow-hidden rounded-2xl border border-white/10 bg-black text-white"
      style={
        {
          "--accent": accent.hex,
          "--accent-soft": accent.soft,
        } as React.CSSProperties
      }
    >
      <TitlebarControls />
      <div className="relative min-h-0 flex-1 overflow-hidden">
        <AnimatePresence mode="wait">
          {showStartup ? (
            <StartupAnimation key="startup" onComplete={() => setShowStartup(false)} />
          ) : showChat ? (
            <Chat key="chat" />
          ) : (
            <Login key="login" />
          )}
        </AnimatePresence>
        <SettingsPanel />
      </div>
    </main>
  );
}

export default App;
