// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils.js";

const badgeVariants = cva(
  "ui:inline-flex ui:items-center ui:gap-1 ui:rounded-sm ui:border ui:px-2 ui:py-0.5 ui:text-xs ui:font-medium",
  {
    variants: {
      variant: {
        default:
          "ui:border-transparent ui:bg-primary ui:text-primary-foreground",
        secondary:
          "ui:border-transparent ui:bg-secondary ui:text-secondary-foreground",
        outline: "ui:border-border ui:text-foreground",
        destructive: "ui:border-border ui:text-destructive",
      },
    },
    defaultVariants: { variant: "outline" },
  },
);

export function Badge({
  className,
  variant,
  ...props
}: ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return (
    <span
      data-slot="badge"
      className={cn(badgeVariants({ variant }), className)}
      {...props}
    />
  );
}
