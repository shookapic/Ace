import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface ToggleShortcut {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
  code: string;
}

export type ShortcutAction = "toggle" | "click_through" | "screenshot";

const OPACITY_STORAGE_KEY = "ace.opacity";
const CAPTURE_STORAGE_KEY = "ace.captureHidden";
const CLICK_THROUGH_STORAGE_KEY = "ace.clickThrough";

function loadStored<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw !== null) return JSON.parse(raw) as T;
  } catch {
    // ignore malformed storage
  }
  return fallback;
}

function store(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // ignore quota / disabled storage
  }
}

const withMods = (code: string): ToggleShortcut => ({
  ctrl: true,
  shift: true,
  alt: false,
  meta: false,
  code,
});

export const DEFAULT_SHORTCUTS: Record<ShortcutAction, ToggleShortcut> = {
  toggle: withMods("KeyA"),
  click_through: withMods("KeyX"),
  screenshot: withMods("KeyS"),
};

export const SHORTCUT_LABELS: Record<ShortcutAction, string> = {
  toggle: "Show / hide Ace",
  click_through: "Toggle click-through",
  screenshot: "Screenshot to composer",
};

export function shortcutLabel(combo: ToggleShortcut): string {
  const parts: string[] = [];
  if (combo.ctrl) parts.push("Ctrl");
  if (combo.shift) parts.push("Shift");
  if (combo.alt) parts.push("Alt");
  if (combo.meta) parts.push("Meta");
  parts.push(combo.code.replace(/^Key/, "").replace(/^Digit/, ""));
  return parts.join("+");
}

const shortcutKey = (action: ShortcutAction) => `ace.shortcut.${action}`;

function loadStoredShortcuts(): Record<ShortcutAction, ToggleShortcut> {
  const result = {} as Record<ShortcutAction, ToggleShortcut>;
  (Object.keys(DEFAULT_SHORTCUTS) as ShortcutAction[]).forEach((action) => {
    result[action] = loadStored(shortcutKey(action), DEFAULT_SHORTCUTS[action]);
  });
  return result;
}

interface WindowState {
  opacity: number;
  captureHidden: boolean;
  clickThrough: boolean;
  hasSeenStartup: boolean;
  shortcuts: Record<ShortcutAction, ToggleShortcut>;
  setOpacity: (alpha: number) => Promise<void>;
  setCaptureHidden: (hidden: boolean) => Promise<void>;
  setClickThrough: (enabled: boolean) => Promise<void>;
  applyStoredWindowPrefs: () => Promise<void>;
  markStartupSeen: () => void;
  applyShortcut: (action: ShortcutAction, combo: ToggleShortcut) => Promise<void>;
  applyAllShortcuts: () => Promise<void>;
}

export const useWindowStore = create<WindowState>((set, get) => ({
  opacity: loadStored(OPACITY_STORAGE_KEY, 1),
  captureHidden: loadStored(CAPTURE_STORAGE_KEY, true),
  clickThrough: loadStored(CLICK_THROUGH_STORAGE_KEY, false),
  hasSeenStartup: false,
  shortcuts: loadStoredShortcuts(),
  setOpacity: async (alpha: number) => {
    set({ opacity: alpha });
    store(OPACITY_STORAGE_KEY, alpha);
    try {
      await invoke("set_window_opacity", { alpha });
    } catch (err) {
      console.error("set_window_opacity failed", err);
    }
  },
  setCaptureHidden: async (hidden: boolean) => {
    set({ captureHidden: hidden });
    store(CAPTURE_STORAGE_KEY, hidden);
    try {
      await invoke("set_capture_hidden", { hidden });
    } catch (err) {
      console.error("set_capture_hidden failed", err);
    }
  },
  setClickThrough: async (enabled: boolean) => {
    set({ clickThrough: enabled });
    store(CLICK_THROUGH_STORAGE_KEY, enabled);
    try {
      await invoke("set_click_through", { enabled });
    } catch (err) {
      console.error("set_click_through failed", err);
    }
  },
  // Re-apply the user's saved window preferences on launch — the Rust side seeds
  // hard defaults (opaque, capture-hidden) at startup, so this makes their last
  // choice win once the window exists.
  applyStoredWindowPrefs: async () => {
    const { opacity, captureHidden, clickThrough } = get();
    try {
      await invoke("set_window_opacity", { alpha: opacity });
      await invoke("set_capture_hidden", { hidden: captureHidden });
      if (clickThrough) await invoke("set_click_through", { enabled: true });
    } catch (err) {
      console.error("failed to apply stored window prefs", err);
    }
  },
  markStartupSeen: () => set({ hasSeenStartup: true }),
  // Rebind one action. Throws if the OS rejects the combo (e.g. already owned by
  // another app), so the recorder UI can surface it.
  applyShortcut: async (action, combo) => {
    await invoke("set_action_shortcut", { action, ...combo });
    set((s) => ({ shortcuts: { ...s.shortcuts, [action]: combo } }));
    store(shortcutKey(action), combo);
  },
  // Re-apply every saved binding on launch (the Rust side only knows defaults).
  applyAllShortcuts: async () => {
    const { shortcuts } = get();
    for (const action of Object.keys(shortcuts) as ShortcutAction[]) {
      try {
        await invoke("set_action_shortcut", { action, ...shortcuts[action] });
      } catch (err) {
        console.error(`failed to apply ${action} shortcut`, err);
      }
    }
  },
}));

// Keep the store in sync when native code (tray item / global shortcut) toggles
// click-through, so the Settings switch reflects reality.
listen<boolean>("window://click-through", (event) => {
  useWindowStore.setState({ clickThrough: event.payload });
  store(CLICK_THROUGH_STORAGE_KEY, event.payload);
});
