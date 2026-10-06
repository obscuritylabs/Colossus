// Adapted from shadcn/ui base-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils.js";

export function Table({ className, ...props }: ComponentProps<"table">) {
  return (
    <div
      data-slot="table-container"
      className="ui:relative ui:w-full ui:overflow-x-auto"
    >
      <table
        data-slot="table"
        className={cn("ui-table ui:w-full ui:text-sm", className)}
        {...props}
      />
    </div>
  );
}
export function TableHeader(props: ComponentProps<"thead">) {
  return <thead data-slot="table-header" {...props} />;
}
export function TableBody(props: ComponentProps<"tbody">) {
  return <tbody data-slot="table-body" {...props} />;
}
export function TableFooter(props: ComponentProps<"tfoot">) {
  return <tfoot data-slot="table-footer" {...props} />;
}
export function TableRow({ className, ...props }: ComponentProps<"tr">) {
  return (
    <tr
      data-slot="table-row"
      className={cn("ui-table-row", className)}
      {...props}
    />
  );
}
export function TableHead({ className, ...props }: ComponentProps<"th">) {
  return (
    <th
      data-slot="table-head"
      className={cn("ui:text-left ui:align-middle", className)}
      {...props}
    />
  );
}
export function TableCell({ className, ...props }: ComponentProps<"td">) {
  return (
    <td
      data-slot="table-cell"
      className={cn("ui:align-middle", className)}
      {...props}
    />
  );
}
export function TableCaption(props: ComponentProps<"caption">) {
  return <caption data-slot="table-caption" {...props} />;
}
