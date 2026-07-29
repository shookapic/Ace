import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import hljs from "highlight.js";
import { CheckIcon, CloseIcon, CopyIcon, ExpandIcon } from "../components/ui/Icons";

interface PreviewPayload {
  name: string;
  language: string;
  content: string;
}

/** Standalone window that renders one file with line numbers + highlighting. */
export function FilePreview() {
  const [file, setFile] = useState<PreviewPayload | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const load = () => invoke<PreviewPayload | null>("get_preview_file").then((f) => setFile(f ?? null));
    load();
    // The window is reused for later files — refresh when told to.
    const un = listen("preview://update", load);
    return () => {
      un.then((f) => f());
    };
  }, []);

  const highlighted = useMemo(() => {
    if (!file) return "";
    try {
      if (file.language && hljs.getLanguage(file.language)) {
        return hljs.highlight(file.content, { language: file.language }).value;
      }
      return hljs.highlightAuto(file.content).value;
    } catch {
      return "";
    }
  }, [file]);

  if (!file) {
    return <div className="flex h-screen items-center justify-center bg-[#0d1117] text-sm text-white/40">Loading…</div>;
  }

  const lineCount = file.content.split("\n").length;
  const win = getCurrentWindow();

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(file.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      /* clipboard blocked — ignore */
    }
  };

  return (
    <div className="flex h-screen flex-col bg-[#0d1117] text-white">
      <div
        data-tauri-drag-region
        className="flex shrink-0 items-center justify-between gap-2 border-b border-white/10 bg-neutral-900 px-3 py-2"
      >
        <div className="min-w-0 truncate text-xs">
          <span className="font-medium text-white/90">{file.name}</span>
          {file.language ? <span className="text-white/40"> · {file.language.toUpperCase()}</span> : null}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={copy}
            className="flex items-center gap-1 rounded-md border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-white/70 transition hover:bg-white/10 hover:text-white/90"
          >
            {copied ? <CheckIcon className="h-3.5 w-3.5" /> : <CopyIcon className="h-3.5 w-3.5" />}
            {copied ? "Copied" : "Copy"}
          </button>
          <button
            type="button"
            onClick={() => win.toggleMaximize()}
            aria-label="Toggle full screen"
            className="rounded-md p-1 text-white/50 transition hover:bg-white/10 hover:text-white"
          >
            <ExpandIcon className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={() => win.close()}
            aria-label="Close"
            className="rounded-md p-1 text-white/50 transition hover:bg-white/10 hover:text-white"
          >
            <CloseIcon className="h-4 w-4" />
          </button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <div className="flex min-w-full font-mono text-[12.5px] leading-relaxed">
          <div className="sticky left-0 z-10 select-none border-r border-white/5 bg-[#0d1117] px-3 py-3 text-right text-white/25">
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>
          <pre className="flex-1 px-3 py-3">
            <code className="hljs bg-transparent p-0" dangerouslySetInnerHTML={{ __html: highlighted }} />
          </pre>
        </div>
      </div>
    </div>
  );
}
