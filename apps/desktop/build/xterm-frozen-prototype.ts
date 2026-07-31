import type { Plugin } from "vite";

const XTERM_MODULE_SUFFIX = "/@xterm/xterm/lib/xterm.mjs";
const MUTABLE_NAMESPACE = "})(Qn||={});";
const NULL_PROTOTYPE_NAMESPACE = "})(Qn||=Object.create(null));";

export function patchXtermForFrozenPrototype(source: string): string {
  const first = source.indexOf(MUTABLE_NAMESPACE);
  if (first === -1 || first !== source.lastIndexOf(MUTABLE_NAMESPACE)) {
    throw new Error(
      "The pinned xterm bundle no longer has the expected KeyCode namespace shape.",
    );
  }
  return source.replace(MUTABLE_NAMESPACE, NULL_PROTOTYPE_NAMESPACE);
}

function isPinnedXtermModule(id: string): boolean {
  const query = id.indexOf("?");
  const path = query === -1 ? id : id.slice(0, query);
  return path.replaceAll("\\", "/").endsWith(XTERM_MODULE_SUFFIX);
}

/// Keep Tauri's prototype freezing enabled while adapting xterm's one mutable
/// namespace object to have no inherited, frozen `toString` property.
export function xtermFrozenPrototypeCompatibility(): Plugin {
  return {
    name: "colossus-xterm-frozen-prototype-compatibility",
    enforce: "pre",
    transform(source, id) {
      if (!isPinnedXtermModule(id)) {
        return null;
      }
      return {
        code: patchXtermForFrozenPrototype(source),
        map: null,
      };
    },
  };
}
