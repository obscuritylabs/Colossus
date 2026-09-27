/** Isolated browser preview data. Never replaces a native or test bridge. */
export function installSetupPreviewApi() {
  if (
    !import.meta.env.DEV ||
    new URLSearchParams(location.search).get("fixture") !== "setup"
  )
    return;
  const host = window as unknown as { __TAURI_INTERNALS__?: unknown };
  if (host.__TAURI_INTERNALS__) return;
  host.__TAURI_INTERNALS__ = {
    invoke: async (command: string) => {
      if (command === "get_provider_presets")
        return [
          {
            id: "codex",
            label: "Codex (ChatGPT subscription)",
            protocol: "codex",
            baseUrl: "https://chatgpt.com/backend-api/codex",
            credentialEnv: null,
          },
          {
            id: "openrouter",
            label: "OpenRouter",
            protocol: "chat_completions",
            baseUrl: "https://openrouter.ai/api/v1",
            credentialEnv: "OPENROUTER_API_KEY",
          },
          {
            id: "openai",
            label: "OpenAI",
            protocol: "responses",
            baseUrl: "https://api.openai.com/v1",
            credentialEnv: "OPENAI_API_KEY",
          },
          {
            id: "ollama",
            label: "Ollama",
            protocol: "chat_completions",
            baseUrl: "http://localhost:11434/v1",
            credentialEnv: null,
          },
          {
            id: "custom-chat",
            label: "Custom Chat Completions",
            protocol: "chat_completions",
            baseUrl: null,
            credentialEnv: null,
          },
          {
            id: "custom-responses",
            label: "Custom Responses",
            protocol: "responses",
            baseUrl: null,
            credentialEnv: null,
          },
        ];
      if (command === "discover_managed_provider_models") {
        await new Promise((resolve) => setTimeout(resolve, 800));
        return {
          credentialId: null,
          models: [
            {
              id: "demo/reasoner",
              display_name: "Example Reasoner",
              description: "Sample model for exploring the setup wizard.",
              context_window_tokens: 128000,
              max_output_tokens: 8192,
              tool_calls: true,
              streaming: true,
              image_inputs: false,
            },
            {
              id: "demo/vision",
              display_name: "Example Vision",
              description: "Sample model with image support.",
              context_window_tokens: 64000,
              max_output_tokens: 4096,
              tool_calls: true,
              streaming: true,
              image_inputs: true,
            },
          ],
        };
      }
      if (
        command === "configure_managed_runtime" ||
        command === "apply_managed_model_configuration"
      )
        return {};
      throw new Error(`Unsupported setup preview command: ${command}`);
    },
  };
}
