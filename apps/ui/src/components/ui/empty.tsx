// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import { cn } from "../../lib/utils.js";

export function Empty({ className, ...props }: ComponentProps<"div">) {
  return (
    <div data-slot="empty" className={cn("ui-empty", className)} {...props} />
  );
}
export function EmptyHeader({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-header"
      className={cn("ui-empty-header", className)}
      {...props}
    />
  );
}
export function EmptyMedia({
  className,
  variant = "default",
  ...props
}: ComponentProps<"div"> & { variant?: "default" | "icon" }) {
  return (
    <div
      data-slot="empty-icon"
      data-variant={variant}
      className={cn("ui-empty-media", className)}
      {...props}
    />
  );
}
export function EmptyTitle({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-title"
      className={cn("ui-empty-title", className)}
      {...props}
    />
  );
}
export function EmptyDescription({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-description"
      className={cn("ui-empty-description", className)}
      {...props}
    />
  );
}
export function EmptyContent({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-content"
      className={cn("ui-empty-content", className)}
      {...props}
    />
  );
}
