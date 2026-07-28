import { getCurrentWindow } from "@tauri-apps/api/window";

export function TitlebarControls() {
  const appWindow = getCurrentWindow();

  return (
    <div className="flex h-8 w-full items-center justify-between px-3 select-none">
      <span
        data-tauri-drag-region
        className="flex-1 text-xs font-semibold tracking-wide text-white/70"
      >
        Ace
      </span>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => appWindow.minimize()}
          className="h-3 w-3 rounded-full bg-yellow-400/80 hover:bg-yellow-400"
          aria-label="Minimize"
        />
        <button
          type="button"
          onClick={() => appWindow.close()}
          className="h-3 w-3 rounded-full bg-red-400/80 hover:bg-red-400"
          aria-label="Close"
        />
      </div>
    </div>
  );
}
