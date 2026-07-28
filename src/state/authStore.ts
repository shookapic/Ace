import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ProviderId = "anthropic" | "openai";

interface AuthStatus {
  provider: ProviderId;
  available: boolean;
  detail?: string;
}

interface LoginResultEvent {
  provider: ProviderId;
  success: boolean;
  error?: string;
}

interface AuthState {
  providers: Record<ProviderId, AuthStatus>;
  justConnected: { provider: ProviderId; key: number } | null;
  refreshStatus: (provider: ProviderId) => Promise<void>;
  startLogin: (provider: ProviderId) => Promise<void>;
  signOut: (provider: ProviderId) => Promise<void>;
}

const initialStatus = (provider: ProviderId): AuthStatus => ({ provider, available: false });

export const useAuthStore = create<AuthState>((set, get) => ({
  providers: {
    anthropic: initialStatus("anthropic"),
    openai: initialStatus("openai"),
  },
  justConnected: null,
  refreshStatus: async (provider) => {
    try {
      const status = await invoke<AuthStatus>("get_auth_status", { provider });
      set((s) => ({ providers: { ...s.providers, [provider]: status } }));
    } catch (err) {
      console.error("get_auth_status failed", err);
    }
  },
  startLogin: async (provider) => {
    // Both providers now resolve asynchronously via the auth://login-result event —
    // OpenAI via its local loopback listener, Anthropic via the `claude setup-token`
    // child process — so there's nothing further to do with the immediate return value.
    await invoke("start_oauth_login", { provider });
  },
  signOut: async (provider) => {
    await invoke("sign_out", { provider });
    await get().refreshStatus(provider);
  },
}));

listen<LoginResultEvent>("auth://login-result", (event) => {
  const { provider, success } = event.payload;
  if (success) {
    useAuthStore.setState({ justConnected: { provider, key: Date.now() } });
    useAuthStore.getState().refreshStatus(provider);
  } else {
    console.error(`oauth login failed for ${provider}: ${event.payload.error}`);
  }
});
