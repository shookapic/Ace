import { AnimatePresence, motion } from "framer-motion";
import { useUpdateStore } from "../state/updateStore";

export function UpdateBanner() {
  const { status, version, progress, dismissed, installUpdate, dismiss } = useUpdateStore();

  const visible = !dismissed && (status === "available" || status === "downloading" || status === "ready");
  if (!visible) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0, y: -12 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -12 }}
        className="absolute left-3 right-3 top-3 z-30 overflow-hidden rounded-xl border border-[color:var(--accent)]/30 bg-neutral-950/95 shadow-2xl backdrop-blur-xl"
      >
        <div className="flex items-center gap-3 px-3 py-2.5">
          <div className="min-w-0 flex-1">
            {status === "downloading" ? (
              <p className="text-xs text-white/80">
                Downloading update… {Math.round(progress * 100)}%
              </p>
            ) : status === "ready" ? (
              <p className="text-xs text-white/80">Update ready — restarting…</p>
            ) : (
              <p className="text-xs text-white/80">
                <span className="font-medium text-white">Ace {version}</span> is available.
              </p>
            )}
          </div>
          {status === "available" ? (
            <>
              <button
                type="button"
                onClick={() => installUpdate()}
                className="shrink-0 rounded-lg bg-[var(--accent)] px-2.5 py-1.5 text-xs font-medium text-black transition hover:opacity-90"
              >
                Update &amp; restart
              </button>
              <button
                type="button"
                onClick={() => dismiss()}
                className="shrink-0 rounded-lg px-2 py-1.5 text-xs text-white/50 transition hover:text-white/80"
              >
                Later
              </button>
            </>
          ) : null}
        </div>
        {status === "downloading" ? (
          <div className="h-0.5 w-full bg-white/10">
            <div
              className="h-full bg-[var(--accent)] transition-[width]"
              style={{ width: `${Math.round(progress * 100)}%` }}
            />
          </div>
        ) : null}
      </motion.div>
    </AnimatePresence>
  );
}
