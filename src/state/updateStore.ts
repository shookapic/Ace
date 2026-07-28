import { create } from "zustand";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type Status = "idle" | "checking" | "available" | "downloading" | "ready" | "uptodate" | "error";

interface UpdateState {
  update: Update | null;
  version: string | null;
  status: Status;
  error: string | null;
  progress: number; // 0..1, meaningful while downloading
  dismissed: boolean;
  checkForUpdate: (manual?: boolean) => Promise<void>;
  installUpdate: () => Promise<void>;
  dismiss: () => void;
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  update: null,
  version: null,
  status: "idle",
  error: null,
  progress: 0,
  dismissed: false,
  checkForUpdate: async (manual = false) => {
    if (get().status === "checking" || get().status === "downloading") return;
    set({ status: "checking", error: null });
    try {
      const update = await check();
      if (update) {
        set({ update, version: update.version, status: "available", dismissed: false });
      } else {
        set({ update: null, version: null, status: "uptodate" });
      }
    } catch (err) {
      // In dev (or if the manifest isn't reachable) this throws — stay quiet on
      // the automatic startup check, surface it only when the user asked.
      set({ status: "error", error: manual ? String(err) : null });
    }
  },
  installUpdate: async () => {
    const update = get().update;
    if (!update) return;
    set({ status: "downloading", progress: 0, error: null });
    try {
      let downloaded = 0;
      let total = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            set({ progress: total > 0 ? downloaded / total : 0 });
            break;
          case "Finished":
            set({ progress: 1 });
            break;
        }
      });
      set({ status: "ready" });
      // Relaunch into the freshly installed version.
      await relaunch();
    } catch (err) {
      set({ status: "error", error: String(err) });
    }
  },
  dismiss: () => set({ dismissed: true }),
}));
