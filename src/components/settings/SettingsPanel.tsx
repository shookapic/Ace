import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { CloseIcon } from "../ui/Icons";
import { useAuthStore, type ProviderId } from "../../state/authStore";
import { useUiStore } from "../../state/uiStore";
import { shortcutLabel, useWindowStore, type ToggleShortcut } from "../../state/windowStore";

const PROVIDER_LABEL: Record<ProviderId, string> = {
  anthropic: "Claude",
  openai: "OpenAI",
};

// Modifier-only key codes we ignore while capturing — a shortcut needs a
// non-modifier key to land on.
const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[10px] font-semibold uppercase tracking-wider text-white/35">
      {children}
    </span>
  );
}

function ShortcutRecorder() {
  const toggleShortcut = useWindowStore((s) => s.toggleShortcut);
  const applyToggleShortcut = useWindowStore((s) => s.applyToggleShortcut);
  const [capturing, setCapturing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const startCapture = () => {
    setError(null);
    setCapturing(true);

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      if (e.code === "Escape") {
        cleanup();
        return;
      }
      if (MODIFIER_CODES.has(e.code)) return;

      const combo: ToggleShortcut = {
        ctrl: e.ctrlKey,
        shift: e.shiftKey,
        alt: e.altKey,
        meta: e.metaKey,
        code: e.code,
      };
      cleanup();
      applyToggleShortcut(combo).catch((err) => setError(String(err)));
    };

    const cleanup = () => {
      window.removeEventListener("keydown", onKeyDown, true);
      setCapturing(false);
    };

    window.addEventListener("keydown", onKeyDown, true);
  };

  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel>Show / hide shortcut</SectionLabel>
      <button
        type="button"
        onClick={startCapture}
        className={`rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
          capturing
            ? "border-[color:var(--accent)] text-[color:var(--accent)]"
            : "border-white/12 text-white/80 hover:border-white/25"
        }`}
      >
        {capturing ? "Press a key combo… (Esc to cancel)" : shortcutLabel(toggleShortcut)}
      </button>
      {error ? <span className="text-[10px] text-red-400">{error}</span> : null}
    </div>
  );
}

export function SettingsPanel() {
  const settingsOpen = useUiStore((s) => s.settingsOpen);
  const closeSettings = useUiStore((s) => s.closeSettings);
  const opacity = useWindowStore((s) => s.opacity);
  const setOpacity = useWindowStore((s) => s.setOpacity);
  const captureHidden = useWindowStore((s) => s.captureHidden);
  const setCaptureHidden = useWindowStore((s) => s.setCaptureHidden);
  const providers = useAuthStore((s) => s.providers);
  const signOut = useAuthStore((s) => s.signOut);

  const connected = (Object.keys(providers) as ProviderId[]).filter(
    (id) => providers[id].available
  );

  return (
    <AnimatePresence>
      {settingsOpen ? (
        <>
          <motion.div
            className="absolute inset-0 z-10 bg-black/50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={closeSettings}
          />
          <motion.div
            className="absolute bottom-3 left-3 right-3 z-20 rounded-2xl border border-white/10 bg-neutral-950/95 p-4 shadow-2xl backdrop-blur-xl"
            initial={{ opacity: 0, y: 16, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 16, scale: 0.97 }}
            transition={{ type: "spring", stiffness: 420, damping: 30 }}
          >
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-sm font-semibold text-white">Settings</h2>
              <button
                type="button"
                onClick={closeSettings}
                aria-label="Close settings"
                className="flex h-6 w-6 items-center justify-center rounded-md text-white/40 transition hover:bg-white/10 hover:text-white"
              >
                <CloseIcon className="h-4 w-4" />
              </button>
            </div>

            <div className="flex flex-col gap-4">
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between">
                  <SectionLabel>Window opacity</SectionLabel>
                  <span className="text-[10px] tabular-nums text-white/40">
                    {Math.round(opacity * 100)}%
                  </span>
                </div>
                <input
                  type="range"
                  min={0.2}
                  max={1}
                  step={0.01}
                  value={opacity}
                  onChange={(e) => setOpacity(Number(e.target.value))}
                  className="h-1 w-full accent-[color:var(--accent)]"
                />
              </div>

              <label className="flex cursor-pointer items-center justify-between">
                <div className="flex flex-col">
                  <span className="text-xs font-medium text-white/90">Hide from screen capture</span>
                  <span className="text-[10px] text-white/40">
                    Stay invisible in screenshots and shares
                  </span>
                </div>
                <input
                  type="checkbox"
                  checked={captureHidden}
                  onChange={(e) => setCaptureHidden(e.target.checked)}
                  className="h-4 w-4 accent-[color:var(--accent)]"
                />
              </label>

              <ShortcutRecorder />

              {connected.length > 0 ? (
                <div className="flex flex-col gap-2 border-t border-white/10 pt-3">
                  <SectionLabel>Account</SectionLabel>
                  {connected.map((id) => (
                    <button
                      key={id}
                      type="button"
                      onClick={() => signOut(id)}
                      className="flex items-center justify-between rounded-lg px-2 py-1.5 text-left text-xs text-white/70 transition hover:bg-white/[0.06] hover:text-white"
                    >
                      <span>Sign out of {PROVIDER_LABEL[id]}</span>
                      <span className="text-white/25">→</span>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          </motion.div>
        </>
      ) : null}
    </AnimatePresence>
  );
}
