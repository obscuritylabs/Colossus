import { sourceTokenColor } from "./tokenColor";
import {
  IconArrowDown,
  IconArrowUp,
  IconFolderSearch,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { GitDiffLine, GitFileDiff, GitFileVersion } from "../../git";
import type { HighlightedLine } from "../../syntax-highlighter";
import { useAppearance } from "../../theme/AppearanceProvider";

export function pairedLines(
  lines: GitDiffLine[],
): { before: GitDiffLine | null; after: GitDiffLine | null }[] {
  const result: { before: GitDiffLine | null; after: GitDiffLine | null }[] =
    [];
  let i = 0;
  while (i < lines.length) {
    if (lines[i]!.kind === "context") {
      result.push({ before: lines[i]!, after: lines[i]! });
      i++;
      continue;
    }
    const removed: GitDiffLine[] = [],
      added: GitDiffLine[] = [];
    while (i < lines.length && lines[i]!.kind !== "context") {
      (lines[i]!.kind === "removed" ? removed : added).push(lines[i]!);
      i++;
    }
    for (let j = 0; j < Math.max(removed.length, added.length); j++)
      result.push({ before: removed[j] ?? null, after: added[j] ?? null });
  }
  return result;
}

function versionMessage(version: GitFileVersion): string | null {
  switch (version.state) {
    case "text":
      return version.content === "" ? "Empty file" : null;
    case "absent":
      return "File does not exist in this version";
    case "binary":
      return "Binary or non-displayable text — content preview unavailable";
    case "too_large":
      return "Preview limit exceeded (256 KiB or 6,000 lines)";
    case "unsupported":
      return "Symbolic links and submodule content are not previewed";
    case "conflict":
      return "Unresolved merge conflict";
    default:
      return "File unavailable — it may have moved or become unreadable";
  }
}

export function DiffViewer({
  diff,
  onReveal,
  onOpenFile,
}: {
  diff: GitFileDiff;
  onReveal: () => void;
  onOpenFile: () => void;
}) {
  const { resolvedColorTheme } = useAppearance();
  const [layout, setLayout] = useState<"inline" | "split">("inline");
  const [current, setCurrent] = useState(0);
  const [tokens, setTokens] = useState<{
    before: HighlightedLine[];
    after: HighlightedLine[];
  } | null>(null);
  const sections = useRef<(HTMLElement | null)[]>([]);
  const before = diff.before.content?.split("\n") ?? [];
  const after = diff.after.content?.split("\n") ?? [];
  const canHighlight = [diff.before, diff.after].every(
    (v) =>
      (v.content?.length ?? 0) < 64_000 &&
      !v.content?.split("\n").some((line) => line.length > 2000),
  );
  useEffect(() => {
    let live = true;
    setCurrent(0);
    setTokens(null);
    if (canHighlight)
      void import("../../syntax-highlighter")
        .then(async ({ highlightSource }) => {
          const [before, after] = await Promise.all([
            highlightSource(
              diff.before.content ?? "",
              diff.language,
              resolvedColorTheme,
            ),
            highlightSource(
              diff.after.content ?? "",
              diff.language,
              resolvedColorTheme,
            ),
          ]);
          if (live) setTokens({ before, after });
        })
        .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [diff, resolvedColorTheme, canHighlight]);
  function navigate(index: number) {
    setCurrent(index);
    sections.current[index]?.scrollIntoView({ block: "start" });
    sections.current[index]?.focus({ preventScroll: true });
  }
  function code(side: "before" | "after", line: number | null) {
    if (line === null) return <code />;
    const highlighted = tokens?.[side][line - 1];
    return (
      <code>
        {highlighted
          ? highlighted.map((token, i) => (
              <span
                key={i}
                style={{
                  color: sourceTokenColor(token.color, resolvedColorTheme),
                }}
              >
                {token.content}
              </span>
            ))
          : (side === "before" ? before : after)[line - 1]}
      </code>
    );
  }
  const hasTextDiff = [diff.before, diff.after].every(
    (v) => v.state === "text" || v.state === "absent",
  );
  return (
    <section className="diff-viewer" aria-label="File diff">
      <div className="diff-toolbar">
        <div className="diff-layout" role="group" aria-label="Diff layout">
          <button
            type="button"
            aria-pressed={layout === "inline"}
            onClick={() => setLayout("inline")}
          >
            Inline
          </button>
          <button
            type="button"
            aria-pressed={layout === "split"}
            onClick={() => setLayout("split")}
          >
            Side by side
          </button>
        </div>
        <span className="diff-totals">
          <span>+{diff.additions}</span> <span>−{diff.deletions}</span>
        </span>
        <div className="diff-navigation">
          <button
            type="button"
            className="icon-button"
            aria-label="Previous change"
            disabled={current === 0 || !diff.hunks.length}
            onClick={() => navigate(current - 1)}
          >
            <IconArrowUp size={16} />
          </button>
          <span role="status">
            {diff.hunks.length
              ? `${current + 1} / ${diff.hunks.length}`
              : "0 changes"}
          </span>
          <button
            type="button"
            className="icon-button"
            aria-label="Next change"
            disabled={current >= diff.hunks.length - 1}
            onClick={() => navigate(current + 1)}
          >
            <IconArrowDown size={16} />
          </button>
        </div>
        <button
          type="button"
          className="icon-button"
          aria-label="Reveal in explorer"
          title="Reveal in explorer"
          onClick={onReveal}
        >
          <IconFolderSearch size={17} />
        </button>
        <button
          type="button"
          className="button secondary compact"
          onClick={onOpenFile}
        >
          Current file
        </button>
      </div>
      <div className="diff-scroll" tabIndex={0} aria-label="Changed lines">
        {diff.previousPath ? (
          <p className="diff-note">Renamed from {diff.previousPath}</p>
        ) : null}
        <p className="diff-note">{diff.note}</p>
        {hasTextDiff && diff.after.state === "absent" ? (
          <p className="diff-note">
            File deleted. Showing the previous version.
          </p>
        ) : hasTextDiff && diff.before.state === "absent" ? (
          <p className="diff-note">New file. No previous version.</p>
        ) : null}
        {!canHighlight ? (
          <p className="diff-note">
            Syntax highlighting is omitted for this file to keep the viewer
            responsive.
          </p>
        ) : null}
        {!hasTextDiff ? (
          <div className="diff-unavailable">
            {(["before", "after"] as const).map((side) => (
              <p key={side}>
                <strong>{side === "before" ? "Before" : "After"}:</strong>{" "}
                {versionMessage(diff[side]) ??
                  "Text available; the other version cannot be compared."}
              </p>
            ))}
          </div>
        ) : null}
        {hasTextDiff && diff.hunks.length === 0 ? (
          <p className="diff-empty">
            {diff.before.state === "absent" && diff.after.content === ""
              ? "New empty file."
              : diff.after.state === "absent" && diff.before.content === ""
                ? "Empty file deleted."
                : "No text changes. The file may have been renamed or its permissions changed."}
          </p>
        ) : null}
        {diff.hunks.map((hunk, index) => (
          <section
            key={index}
            ref={(node) => {
              sections.current[index] = node;
            }}
            tabIndex={-1}
            aria-label={`Change ${index + 1}`}
            className={`diff-hunk diff-${layout}${current === index ? " is-current" : ""}`}
          >
            <h3>
              @@ −{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},
              {hunk.newLines} @@
            </h3>
            {layout === "split" ? (
              <>
                <div className="diff-split-labels">
                  <span>Before</span>
                  <span>After</span>
                </div>
                {pairedLines(hunk.lines).map((row, i) => (
                  <div className="diff-split-row" key={i}>
                    {(["before", "after"] as const).map((side) => {
                      const line = row[side],
                        number =
                          side === "before" ? line?.oldLine : line?.newLine;
                      return (
                        <div
                          className={`diff-cell ${line?.kind ?? "spacer"}`}
                          key={side}
                        >
                          <span className="diff-line-number">{number}</span>
                          <span
                            className="diff-marker"
                            aria-label={
                              line?.kind === "removed"
                                ? "Removed"
                                : line?.kind === "added"
                                  ? "Added"
                                  : undefined
                            }
                          >
                            {line?.kind === "removed"
                              ? "−"
                              : line?.kind === "added"
                                ? "+"
                                : " "}
                          </span>
                          {code(side, number ?? null)}
                        </div>
                      );
                    })}
                  </div>
                ))}
              </>
            ) : (
              hunk.lines.map((line, i) => (
                <div className={`diff-inline-row ${line.kind}`} key={i}>
                  <span className="diff-line-number" title="Old line">
                    {line.oldLine}
                  </span>
                  <span className="diff-line-number" title="New line">
                    {line.newLine}
                  </span>
                  <span
                    className="diff-marker"
                    aria-label={
                      line.kind === "removed"
                        ? "Removed"
                        : line.kind === "added"
                          ? "Added"
                          : undefined
                    }
                  >
                    {line.kind === "removed"
                      ? "−"
                      : line.kind === "added"
                        ? "+"
                        : " "}
                  </span>
                  {code(
                    line.kind === "removed" ? "before" : "after",
                    line.kind === "removed" ? line.oldLine : line.newLine,
                  )}
                </div>
              ))
            )}
          </section>
        ))}
        {hasTextDiff &&
          (["before", "after"] as const).map((side) =>
            diff[side].content && !diff[side].content!.endsWith("\n") ? (
              <p key={side} className="diff-note">
                {side === "before" ? "Before" : "After"}: no newline at end of
                file.
              </p>
            ) : null,
          )}
        {diff.truncated ? (
          <p className="diff-limit" role="status">
            This diff is truncated at 2,000 displayed lines or 128 changes.
            Totals include the whole bounded comparison.
          </p>
        ) : null}
      </div>
    </section>
  );
}
