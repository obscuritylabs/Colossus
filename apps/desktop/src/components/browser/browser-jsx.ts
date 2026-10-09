import * as React from "react";

// Keep browser JSX compact while retaining React 19's scoped JSX types.
export function element(...args: Parameters<typeof React.createElement>) {
  return React.createElement(...args);
}
export declare namespace element {
  export import JSX = React.JSX;
}
export const Fragment = React.Fragment;
