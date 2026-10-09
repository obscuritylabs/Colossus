// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import * as MenuPrimitive from "@radix-ui/react-dropdown-menu";
import { IconCheck } from "@tabler/icons-react";
import { cn } from "../../lib/utils.js";

export const DropdownMenu = MenuPrimitive.Root;
export const DropdownMenuTrigger = MenuPrimitive.Trigger;
export const DropdownMenuGroup = MenuPrimitive.Group;

export function DropdownMenuContent({
  align = "start",
  sideOffset = 4,
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Content>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        {...props}
        align={align}
        sideOffset={sideOffset}
        data-slot="dropdown-menu-content"
        className={cn("ui-menu-content", className)}
      />
    </MenuPrimitive.Portal>
  );
}
export function DropdownMenuLabel({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Label>) {
  return (
    <MenuPrimitive.Label
      {...props}
      className={cn("ui-menu-label", className)}
    />
  );
}
export function DropdownMenuItem({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Item>) {
  return (
    <MenuPrimitive.Item {...props} className={cn("ui-menu-item", className)} />
  );
}
export function DropdownMenuCheckboxItem({
  children,
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.CheckboxItem>) {
  return (
    <MenuPrimitive.CheckboxItem
      {...props}
      className={cn("ui-menu-item ui-menu-checkbox", className)}
    >
      {children}
      <MenuPrimitive.ItemIndicator>
        <IconCheck size={16} aria-hidden="true" />
      </MenuPrimitive.ItemIndicator>
    </MenuPrimitive.CheckboxItem>
  );
}
