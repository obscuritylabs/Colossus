import assert from "node:assert/strict";
import { test } from "node:test";
import { canPreviewMermaid } from "../src/markdown/mermaid-security.ts";

test("ordinary offline flow, sequence and state diagrams can be previewed", () => {
  for (const source of [
    "flowchart LR\n A[User] --> B[Runtime]",
    "graph TD\n A -->|Ready| B",
    "sequenceDiagram\n User->>Runtime: Start\n Runtime-->>User: Done",
    "stateDiagram-v2\n [*] --> Ready\n Ready --> Running",
    "classDiagram\n Animal <|-- Duck",
    "erDiagram\n USER ||--o{ CHAT : owns",
    "gantt\n dateFormat YYYY-MM-DD\n Work :a1, 2026-10-21, 1d",
    'pie title Outcomes\n "Ready" : 3\n "Running" : 1',
  ])
    assert.equal(canPreviewMermaid(source), true, source);
});

test("configuration, styling, interactive and remote diagram content is rejected before rendering", () => {
  for (const source of [
    "---\nconfig:\n securityLevel: loose\n---\nflowchart LR\n A --> B",
    'flowchart LR\n %%{init: {"securityLevel":"loose"}}%%\n A --> B',
    'flowchart LR\n A --> B\n click A "https://example.invalid/secret"',
    "flowchart LR\n A --> B; style A fill:red",
    "flowchart LR\n A --> B\n classDef default fill:url(https://example.invalid)",
    "sequenceDiagram\n box url(/private) Group\n participant User\n end",
    'flowchart LR\n A@{ img: "https://example.invalid/image" }',
    'flowchart LR\n A["<img src=//example.invalid/image>"]',
    'flowchart LR\n A["#60;img src=x#62;"]',
    'flowchart LR\n A["data:image/svg+xml,payload"]',
    "flowchart LR\n A:::hidden --> B",
    "flowchart LR\n A[fa:fa-user] --> B",
    "flowchart LR\n A\u0000 --> B",
    `flowchart LR\n A[${"x".repeat(6_000)}]`,
    `flowchart LR\n${"A;".repeat(201)}`,
    "architecture-beta\n group api(cloud)[API]",
  ])
    assert.equal(canPreviewMermaid(source), false, source.slice(0, 80));
});
