import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export interface ToggleShortcut {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
  code: string;
}

const SHORTCUT_STORAGE_KEY = "ace.toggleShortcut";

export const DEFAULT_TOGGLE_SHORTCUT: ToggleShortcut = {
  ctrl: true,
  shift: true,
  alt: false,
  meta: false,
  code: "KeyA",
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

function loadStoredShortcut(): ToggleShortcut {
  try {
    const raw = localStorage.getItem(SHORTCUT_STORAGE_KEY);
    if (raw) return { ...DEFAULT_TOGGLE_SHORTCUT, ...JSON.parse(raw) };
  } catch {
    // ignore malformed storage
  }
  return DEFAULT_TOGGLE_SHORTCUT;
}

interface WindowState {
  opacity: number;
  captureHidden: boolean;
  hasSeenStartup: boolean;
  toggleShortcut: ToggleShortcut;
  setOpacity: (alpha: number) => Promise<void>;
  setCaptureHidden: (hidden: boolean) => Promise<void>;
  markStartupSeen: () => void;
  applyToggleShortcut: (combo: ToggleShortcut) => Promise<void>;
}

export const useWindowStore = create<WindowState>((set) => ({
  opacity: 1,
  captureHidden: true,
  hasSeenStartup: false,
  toggleShortcut: loadStoredShortcut(),
  setOpacity: async (alpha: number) => {
    set({ opacity: alpha });
    try {
      await invoke("set_window_opacity", { alpha });
    } catch (err) {
      console.error("set_window_opacity failed", err);
    }
  },
  setCaptureHidden: async (hidden: boolean) => {
    set({ captureHidden: hidden });
    try {
      await invoke("set_capture_hidden", { hidden });
    } catch (err) {
      console.error("set_capture_hidden failed", err);
    }
  },
  markStartupSeen: () => set({ hasSeenStartup: true }),
  applyToggleShortcut: async (combo: ToggleShortcut) => {
    await invoke("set_toggle_shortcut", { ...combo });
    set({ toggleShortcut: combo });
    localStorage.setItem(SHORTCUT_STORAGE_KEY, JSON.stringify(combo));
  },
}));
