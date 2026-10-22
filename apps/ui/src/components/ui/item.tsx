// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import { Slot } from "@radix-ui/react-slot";
import { cn } from "../../lib/utils.js";

export function ItemGroup({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      role="list"
      data-slot="item-group"
      className={cn("ui-item-group", className)}
      {...props}
    />
  );
}
export function Item({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: ComponentProps<"div"> & {
  variant?: "default" | "outline" | "muted";
  size?: "default" | "sm";
  asChild?: boolean;
}) {
  const Component = asChild ? Slot : "div";
  return (
    <Component
      data-slot="item"
      data-variant={variant}
      data-size={size}
      className={cn("ui-item", className)}
      {...props}
    />
  );
}
export function ItemMedia({
  className,
  variant = "default",
  ...props
}: ComponentProps<"div"> & { variant?: "default" | "icon" | "image" }) {
  return (
    <div
      data-slot="item-media"
      data-variant={variant}
      className={cn("ui-item-media", className)}
      {...props}
    />
  );
}
export function ItemContent({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="item-content"
      className={cn("ui-item-content", className)}
      {...props}
    />
  );
}
export function ItemTitle({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="item-title"
      className={cn("ui-item-title", className)}
      {...props}
    />
  );
}
export function ItemDescription({ className, ...props }: ComponentProps<"p">) {
  return (
    <p
      data-slot="item-description"
      className={cn("ui-item-description", className)}
      {...props}
    />
  );
}
export function ItemActions({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="item-actions"
      className={cn("ui-item-actions", className)}
      {...props}
    />
  );
}
