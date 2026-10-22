export function patchMermaidForColossus(
  source: string,
  id: string,
): string | null;

export function mermaidViteCompatibility(): {
  name: string;
  enforce: "pre";
  transform(source: string, id: string): { code: string; map: null } | null;
};

export function mermaidEsbuildCompatibility(): {
  name: string;
  setup(build: {
    onLoad(
      options: { filter: RegExp },
      callback: (args: {
        path: string;
      }) => Promise<{ contents: string; loader: "js" } | null>,
    ): void;
  }): void;
};
