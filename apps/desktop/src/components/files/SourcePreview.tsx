import { sourceTokenColor } from "./tokenColor";
import { useEffect, useMemo, useState } from "react";
import type { WorkspaceFile } from "../../types";
import type { ResolvedColorTheme } from "../../theme/appearance";
async function highlight(
  content: string,
  language: string,
  colorTheme: ResolvedColorTheme,
): Promise<import("../../syntax-highlighter").HighlightedLine[]> {
  const { highlightSource } = await import("../../syntax-highlighter");
  return highlightSource(content, language, colorTheme);
}

export function HighlightedCode({
  file,
  colorTheme,
}: {
  file: WorkspaceFile;
  colorTheme: ResolvedColorTheme;
}) {
  const sourceLines = useMemo(() => file.content.split("\n"), [file.content]);
  const bounded = sourceLines.length > 6000;
  const content = useMemo(
    () => sourceLines.slice(0, 6000).join("\n"),
    [sourceLines],
  );
  const canHighlight =
    content.length < 64_000 && !sourceLines.some((line) => line.length > 2000);
  const [lines, setLines] = useState<
    import("../../syntax-highlighter").HighlightedLine[] | null
  >(null);

  useEffect(() => {
    let current = true;
    setLines(null);
    if (canHighlight)
      void highlight(content, file.language, colorTheme)
        .then((highlighted) => {
          if (current) {
            setLines(highlighted);
          }
        })
        .catch(() => {
          if (current) {
            setLines(
              content
                .split("\n")
                .map((line) => [{ content: line, color: undefined }]),
            );
          }
        });
    return () => {
      current = false;
    };
  }, [colorTheme, content, file.language, canHighlight]);

  const visibleLines =
    lines ??
    content.split("\n").map((line) => [{ content: line, color: undefined }]);

  return (
    <div
      className="file-code-scroll"
      aria-label={`${file.name} source preview`}
      tabIndex={0}
    >
      {bounded ? (
        <p className="diff-limit" role="status">
          Showing the first 6,000 lines. This preview is truncated.
        </p>
      ) : null}
      {!canHighlight ? (
        <p className="diff-note">
          Syntax highlighting is omitted for this file to keep the viewer
          responsive.
        </p>
      ) : null}
      <div className="file-code" role="presentation">
        {visibleLines.map((line, index) => (
          <div className="file-code-line" key={`${index}-${line.length}`}>
            <span className="file-line-number" aria-hidden="true">
              {index + 1}
            </span>
            <code>
              {line.length === 0 ? (
                <span>&nbsp;</span>
              ) : (
                line.map((token, tokenIndex) => (
                  <span
                    key={`${tokenIndex}-${token.content.length}`}
                    style={
                      token.color === undefined
                        ? undefined
                        : { color: sourceTokenColor(token.color, colorTheme) }
                    }
                  >
                    {token.content}
                  </span>
                ))
              )}
            </code>
          </div>
        ))}
      </div>
    </div>
  );
}
