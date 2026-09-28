import {
  IconFile,
  IconFileCode,
  IconFileText,
  IconSearch,
  IconX,
} from "@tabler/icons-react";

export function FileName({ path }: { path: string }) {
  const split = path.lastIndexOf("/");
  const name = path.slice(split + 1);
  const directory = split === -1 ? "" : path.slice(0, split);
  const FileIcon = /\.(tsx?|jsx?|rs|py|go|css|json|ya?ml|toml)$/.test(name)
    ? IconFileCode
    : /\.(md|txt|rst)$/.test(name)
      ? IconFileText
      : IconFile;
  return (
    <>
      <FileIcon size={16} className="git-file-icon" aria-hidden="true" />
      <span className="git-file-name" title={path}>
        <strong>{name}</strong>
        {directory ? <small>{directory}</small> : null}
      </span>
    </>
  );
}

export function StatusBadge({ status }: { status: string }) {
  const code =
    (
      {
        modified: "M",
        added: "A",
        deleted: "D",
        renamed: "R",
        new: "U",
        conflict: "!",
        "type changed": "T",
        unreadable: "?",
      } as Record<string, string>
    )[status] ?? "?";
  return (
    <span
      className={`git-status-badge is-${code === "!" ? "conflict" : code.toLowerCase()}`}
      title={status}
      aria-label={status}
    >
      {code}
    </span>
  );
}

export function GitFilter({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
}) {
  return (
    <div className="git-filter">
      <IconSearch size={15} aria-hidden="true" />
      <input
        type="search"
        aria-label={label}
        placeholder={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      {value ? (
        <button
          type="button"
          className="icon-button"
          aria-label={`Clear ${label.toLowerCase()}`}
          onClick={() => onChange("")}
        >
          <IconX size={14} />
        </button>
      ) : null}
    </div>
  );
}
