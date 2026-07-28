import type { ProviderId } from "./state/authStore";

// The UI's accent reflects who you're talking to: Claude's clay, OpenAI's green.
// Switching provider shifts every accent surface (send button, focus ring,
// active states) so the interface always signals which model is live.
export const PROVIDER_ACCENT: Record<ProviderId, { hex: string; soft: string }> = {
  anthropic: { hex: "#D97757", soft: "rgba(217, 119, 87, 0.16)" },
  openai: { hex: "#10A37F", soft: "rgba(16, 163, 127, 0.16)" },
};

export const NEUTRAL_ACCENT = { hex: "#8b8b94", soft: "rgba(255, 255, 255, 0.08)" };
