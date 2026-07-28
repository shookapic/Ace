/** Three dots that bounce left-to-right while the assistant is composing. */
export function TypingDots() {
  return (
    <span
      className="ace-typing inline-flex items-center gap-1 py-1.5 align-middle text-current"
      role="status"
      aria-label="Assistant is typing"
    >
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
    </span>
  );
}
