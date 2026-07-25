import type {
  WorkspaceDirectory,
  WorkspaceEntry,
  WorkspaceFile,
} from "../types";

function directory(name: string, path: string): WorkspaceEntry {
  return { name, path, kind: "directory", sizeBytes: null };
}

function file(name: string, path: string, sizeBytes: number): WorkspaceEntry {
  return { name, path, kind: "file", sizeBytes };
}

const DIRECTORIES: Readonly<Record<string, readonly WorkspaceEntry[]>> = {
  "": [
    directory("apps", "apps"),
    directory("crates", "crates"),
    directory("docs", "docs"),
    directory("scripts", "scripts"),
    file("AGENTS.md", "AGENTS.md", 1_204),
    file("Cargo.toml", "Cargo.toml", 4_892),
    file("README.md", "README.md", 3_426),
  ],
  apps: [directory("desktop", "apps/desktop")],
  "apps/desktop": [
    directory("src", "apps/desktop/src"),
    directory("src-tauri", "apps/desktop/src-tauri"),
    file("package.json", "apps/desktop/package.json", 1_481),
    file("vite.config.ts", "apps/desktop/vite.config.ts", 672),
  ],
  "apps/desktop/src": [
    directory("components", "apps/desktop/src/components"),
    file("App.tsx", "apps/desktop/src/App.tsx", 54_732),
    file("api.ts", "apps/desktop/src/api.ts", 6_204),
    file("styles.css", "apps/desktop/src/styles.css", 38_210),
    file("types.ts", "apps/desktop/src/types.ts", 12_406),
  ],
  "apps/desktop/src/components": [
    file(
      "ProductRail.tsx",
      "apps/desktop/src/components/ProductRail.tsx",
      3_160,
    ),
    file(
      "WorkSidebar.tsx",
      "apps/desktop/src/components/WorkSidebar.tsx",
      7_298,
    ),
    file(
      "WorkSurface.tsx",
      "apps/desktop/src/components/WorkSurface.tsx",
      9_826,
    ),
  ],
  "apps/desktop/src-tauri": [
    directory("src", "apps/desktop/src-tauri/src"),
    file("Cargo.toml", "apps/desktop/src-tauri/Cargo.toml", 1_628),
    file("tauri.conf.json", "apps/desktop/src-tauri/tauri.conf.json", 1_704),
  ],
  "apps/desktop/src-tauri/src": [
    file("lib.rs", "apps/desktop/src-tauri/src/lib.rs", 2_284),
    file(
      "workspace_files.rs",
      "apps/desktop/src-tauri/src/workspace_files.rs",
      17_402,
    ),
  ],
  crates: [
    directory("colossus-api", "crates/colossus-api"),
    directory("colossus-runtime", "crates/colossus-runtime"),
    directory("colossus-sdk", "crates/colossus-sdk"),
  ],
  "crates/colossus-api": [
    directory("src", "crates/colossus-api/src"),
    file("Cargo.toml", "crates/colossus-api/Cargo.toml", 1_340),
  ],
  "crates/colossus-api/src": [
    file("lib.rs", "crates/colossus-api/src/lib.rs", 2_120),
    file("runs.rs", "crates/colossus-api/src/runs.rs", 28_340),
  ],
  docs: [
    directory("develop", "docs/develop"),
    file("index.md", "docs/index.md", 2_804),
  ],
  "docs/develop": [
    file("architecture.md", "docs/develop/architecture.md", 5_480),
    file(
      "security-architecture.md",
      "docs/develop/security-architecture.md",
      24_612,
    ),
  ],
  scripts: [
    file("check_crate_roots.sh", "scripts/check_crate_roots.sh", 2_306),
  ],
};

const CONTENT: Readonly<Record<string, Omit<WorkspaceFile, "path">>> = {
  "README.md": {
    name: "README.md",
    language: "markdown",
    sizeBytes: 3_426,
    lineCount: 18,
    content: `# Colossus

Colossus is a security-first agent runtime with a native Operations Studio.

## Development

\`\`\`sh
cargo xtask dev
cd apps/desktop && npm run dev
\`\`\`

The Rust runtime owns model, tool, policy, and state behavior. Desktop remains
an interface over narrow, typed native capabilities.
`,
  },
  "Cargo.toml": {
    name: "Cargo.toml",
    language: "toml",
    sizeBytes: 4_892,
    lineCount: 15,
    content: `[workspace]
resolver = "3"
members = [
  "crates/colossus-agent",
  "crates/colossus-api",
  "crates/colossus-runtime",
  "crates/colossus-sdk",
]

[workspace.package]
edition = "2024"
rust-version = "1.96"
license = "Apache-2.0"
`,
  },
  "apps/desktop/src/components/WorkSurface.tsx": {
    name: "WorkSurface.tsx",
    language: "tsx",
    sizeBytes: 9_826,
    lineCount: 19,
    content: `import {
  IconFiles,
  IconFolderOpen,
} from "@tabler/icons-react";

type WorkDrawer = "files" | "artifacts" | null;

export function WorkSurface() {
  const [activeDrawer, setActiveDrawer] =
    useState<WorkDrawer>(null);

  return (
    <button
      aria-controls="work-side-drawer"
      aria-expanded={activeDrawer === "files"}
      onClick={() => setActiveDrawer("files")}
    >
      <IconFiles /> Files
    </button>
  );
}
`,
  },
  "apps/desktop/src-tauri/src/workspace_files.rs": {
    name: "workspace_files.rs",
    language: "rust",
    sizeBytes: 17_402,
    lineCount: 24,
    content: `use std::{
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
};

const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_FILE_BYTES: u64 = 256 * 1_024;

fn read_file(root: &Path, relative: &str) -> Result<WorkspaceFileDto, CommandErrorDto> {
    let candidate = resolve_relative(root, relative, false)?;
    let before = fs::symlink_metadata(&candidate)
        .map_err(|_| workspace_read_error())?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(preview_unavailable());
    }

    let file = open_file_without_following(&candidate)?;
    // The native boundary returns bounded UTF-8 text only.
    read_bounded_preview(file, relative)
}
`,
  },
  "apps/desktop/src/styles.css": {
    name: "styles.css",
    language: "css",
    sizeBytes: 38_210,
    lineCount: 17,
    content: `.workspace-files-drawer {
  display: grid;
  height: 100%;
  min-width: 0;
  min-height: 0;
  grid-template-columns: minmax(156px, 35%) minmax(0, 1fr);
}

.file-explorer {
  overflow: hidden;
  border-right: 1px solid var(--border);
  background: var(--sidebar);
}
`,
  },
};

export async function listFixtureWorkspaceDirectory(
  _workspaceId: string,
  path = "",
): Promise<WorkspaceDirectory> {
  return {
    path,
    entries: [...(DIRECTORIES[path] ?? [])],
    truncated: false,
    excludedCount: path === "" ? 5 : 0,
  };
}

export async function readFixtureWorkspaceFile(
  _workspaceId: string,
  path: string,
): Promise<WorkspaceFile> {
  const known = CONTENT[path];
  if (known !== undefined) {
    return { ...known, path };
  }
  const name = path.split("/").at(-1) ?? path;
  return {
    name,
    path,
    language: languageForFixture(name),
    sizeBytes: 1_024,
    lineCount: 5,
    content: `// Deterministic preview fixture for ${path}\n\nexport const ready = true;\n`,
  };
}

function languageForFixture(name: string): string {
  const extension = name.split(".").at(-1)?.toLowerCase();
  return (
    {
      md: "markdown",
      rs: "rust",
      toml: "toml",
      ts: "typescript",
      tsx: "tsx",
      json: "json",
      css: "css",
      sh: "bash",
    }[extension ?? ""] ?? "text"
  );
}
