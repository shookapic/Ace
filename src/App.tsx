import { useEffect, useState } from "react";
import { AnimatePresence } from "framer-motion";
import { TitlebarControls } from "./components/window/TitlebarControls";
import { SettingsPanel } from "./components/settings/SettingsPanel";
import { UpdateBanner } from "./components/UpdateBanner";
import { StartupAnimation } from "./components/branding/StartupAnimation";
import { Login } from "./routes/Login";
import { Chat } from "./routes/Chat";
import { useAuthStore } from "./state/authStore";
import { useChatStore } from "./state/chatStore";
import { useUpdateStore } from "./state/updateStore";
import { useWindowStore } from "./state/windowStore";
import { NEUTRAL_ACCENT, PROVIDER_ACCENT } from "./theme";

function App() {
  const [showStartup, setShowStartup] = useState(true);
  const [checkedAuth, setCheckedAuth] = useState(false);
  const providers = useAuthStore((s) => s.providers);
  const refreshStatus = useAuthStore((s) => s.refreshStatus);
  const activeProvider = useChatStore((s) => s.provider);
  const hydrateConversations = useChatStore((s) => s.hydrateConversations);
  const applyAllShortcuts = useWindowStore((s) => s.applyAllShortcuts);
  const applyStoredWindowPrefs = useWindowStore((s) => s.applyStoredWindowPrefs);
  const checkForUpdate = useUpdateStore((s) => s.checkForUpdate);
  const accent = activeProvider ? PROVIDER_ACCENT[activeProvider] : NEUTRAL_ACCENT;

  useEffect(() => {
    Promise.all([refreshStatus("openai"), refreshStatus("anthropic")]).finally(() =>
      setCheckedAuth(true)
    );
  }, [refreshStatus]);

  useEffect(() => {
    applyAllShortcuts();
    // Restore the user's saved opacity / capture-hide / click-through choices,
    // overriding the Rust startup defaults.
    applyStoredWindowPrefs();
    // Load saved chats from the encrypted on-disk store.
    hydrateConversations();
    // Silent auto-check on launch; the banner only appears if something's newer.
    checkForUpdate();
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
        <UpdateBanner />
        <SettingsPanel />
      </div>
    </main>
  );
}

export default App;
