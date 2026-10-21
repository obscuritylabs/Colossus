export type CapabilityKind = "tools" | "mcp" | "search" | "plugins";

export interface CapabilityEntry {
  name: string;
  description: string;
  detail: string;
  status: string;
}

export interface CapabilitySection {
  kind: CapabilityKind;
  available: boolean;
  message: string;
  entries: CapabilityEntry[];
}

export interface CapabilityInventory {
  targetId: string;
  sections: CapabilitySection[];
}
