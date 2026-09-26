export const setupSteps = [
  {
    id: "desktop",
    label: "Desktop",
    title: "Welcome to Colossus",
    description:
      "Make yourself at home. Choose how the app looks before setting up your workspace.",
  },
  {
    id: "workspace",
    label: "Workspace",
    title: "Choose your first workspace",
    description:
      "Start with a project folder. Your models and tool settings will be saved to this workspace.",
  },
  {
    id: "provider",
    label: "Provider",
    title: "Connect a provider",
    description:
      "Choose where your models run. Use a subscription, an API service, or a local server.",
  },
  {
    id: "model",
    label: "Model",
    title: "Choose a model",
    description:
      "Browse your provider’s models or enter a model ID to get started.",
  },
  {
    id: "start",
    label: "Start",
    title: "Ready when you are",
    description: "Review your setup and choose what Colossus can access.",
  },
] as const;

export type SetupStep = (typeof setupSteps)[number]["id"];

export function validProviderUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      (url.protocol === "https:" || url.protocol === "http:") &&
      url.hostname !== "" &&
      url.username === "" &&
      url.password === ""
    );
  } catch {
    return false;
  }
}
