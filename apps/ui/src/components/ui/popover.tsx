// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import { cn } from "../../lib/utils.js";

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;

export function PopoverContent({
  className,
  align = "start",
  sideOffset = 6,
  portalContainer,
  ...props
}: ComponentProps<typeof PopoverPrimitive.Content> & {
  portalContainer?: ComponentProps<typeof PopoverPrimitive.Portal>["container"];
}) {
  return (
    <PopoverPrimitive.Portal
      {...(portalContainer ? { container: portalContainer } : {})}
    >
      <PopoverPrimitive.Content
        {...props}
        align={align}
        sideOffset={sideOffset}
        data-slot="popover-content"
        className={cn("ui-popover-content", className)}
      />
    </PopoverPrimitive.Portal>
  );
}
