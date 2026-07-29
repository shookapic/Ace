import { memo, useRef, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CheckIcon, CopyIcon } from "./Icons";

// Open links in the user's real browser instead of navigating the app's own
// webview — which holds privileged Tauri IPC. Only http(s)/mailto are honored.
function SafeLink({ href, children }: { href?: string; children?: ReactNode }) {
  return (
    <a
      href={href}
      rel="noopener noreferrer"
      onClick={(e) => {
        e.preventDefault();
        if (href && /^(https?:|mailto:)/i.test(href)) openUrl(href).catch(() => {});
      }}
    >
      {children}
    </a>
  );
}

/** Small copy-to-clipboard button that flips to a check for a moment. */
function CopyButton({ getText, className = "" }: { getText: () => string; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(getText());
          setCopied(true);
          setTimeout(() => setCopied(false), 1400);
        } catch {
          /* clipboard blocked — ignore */
        }
      }}
      aria-label={copied ? "Copied" : "Copy"}
      className={`flex items-center gap-1 rounded-md border border-white/10 bg-white/5 px-1.5 py-1 text-[11px] text-white/60 transition hover:bg-white/10 hover:text-white/90 ${className}`}
    >
      {copied ? <CheckIcon className="h-3.5 w-3.5" /> : <CopyIcon className="h-3.5 w-3.5" />}
    </button>
  );
}

/** A fenced code block: gray panel, highlighted body, copy button. */
function CodeBlock({ children }: { children?: ReactNode }) {
  const ref = useRef<HTMLPreElement>(null);
  return (
    <div className="group relative my-2 overflow-hidden rounded-lg border border-white/10 bg-[#0d1117]">
      <CopyButton
        getText={() => ref.current?.textContent ?? ""}
        className="absolute right-1.5 top-1.5 z-10 opacity-0 group-hover:opacity-100 focus:opacity-100"
      />
      <pre ref={ref} className="overflow-x-auto px-3 py-2.5 text-[12.5px] leading-relaxed">
        {children}
      </pre>
    </div>
  );
}

// Memoized: react-markdown + rehype-highlight is expensive, and the parent chat
// view re-renders on every keystroke / stream chunk. With a stable `content`
// string this skips re-parsing every other message in the thread — the fix for
// stutter on large conversations.
export const MarkdownMessage = memo(function MarkdownMessage({ content }: { content: string }) {
  return (
    <div className="md-body">
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[[rehypeHighlight, { detect: true, ignoreMissing: true }]]}
      components={{
        a: ({ href, children }) => <SafeLink href={href}>{children}</SafeLink>,
        pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
        code: ({ className, children, ...rest }) => {
          const isBlock = /\b(hljs|language-)/.test(className ?? "");
          if (isBlock) {
            return (
              <code className={className} {...rest}>
                {children}
              </code>
            );
          }
          return (
            <code className="rounded bg-white/10 px-1 py-0.5 text-[0.85em] text-white/90">{children}</code>
          );
        },
      }}
    >
      {content}
    </ReactMarkdown>
    </div>
  );
});

export { CopyButton };
