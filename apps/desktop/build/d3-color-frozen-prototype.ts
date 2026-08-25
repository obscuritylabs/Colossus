import type { Plugin } from "vite";

const D3_COLOR_MODULE_SUFFIX = "/d3-color/src/define.js";
const INHERITED_CONSTRUCTOR_ASSIGNMENT =
  "  prototype.constructor = constructor;";
const OWN_CONSTRUCTOR_DEFINITION = `  Object.defineProperty(prototype, "constructor", {
    value: constructor,
    writable: true,
    configurable: true,
  });`;

export function patchD3ColorForFrozenPrototype(source: string): string {
  const first = source.indexOf(INHERITED_CONSTRUCTOR_ASSIGNMENT);
  if (
    first === -1 ||
    first !== source.lastIndexOf(INHERITED_CONSTRUCTOR_ASSIGNMENT)
  ) {
    throw new Error(
      "The pinned d3-color module no longer has the expected constructor assignment shape.",
    );
  }
  return source.replace(
    INHERITED_CONSTRUCTOR_ASSIGNMENT,
    OWN_CONSTRUCTOR_DEFINITION,
  );
}

function isPinnedD3ColorModule(id: string): boolean {
  const query = id.indexOf("?");
  const path = query === -1 ? id : id.slice(0, query);
  return path.replaceAll("\\", "/").endsWith(D3_COLOR_MODULE_SUFFIX);
}

/// Keep Tauri's prototype freezing enabled while adapting d3-color's one
/// inherited `constructor` assignment into an equivalent own property.
export function d3ColorFrozenPrototypeCompatibility(): Plugin {
  return {
    name: "colossus-d3-color-frozen-prototype-compatibility",
    enforce: "pre",
    transform(source, id) {
      if (!isPinnedD3ColorModule(id)) {
        return null;
      }
      return {
        code: patchD3ColorForFrozenPrototype(source),
        map: null,
      };
    },
  };
}
