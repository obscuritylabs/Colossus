// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils.js";

const buttonVariants = cva("ui-button", {
  variants: {
    variant: {
      default: "ui-button--primary",
      outline: "ui-button--secondary",
      secondary: "ui-button--secondary",
      ghost: "ui-button--tertiary",
      destructive: "ui-button--danger",
      link: "ui-button--link",
    },
    size: {
      default: "",
      sm: "",
      lg: "ui-button--large",
      icon: "ui-button--icon",
    },
  },
  defaultVariants: { variant: "default", size: "default" },
});

function Button({
  className,
  variant = "default",
  size = "default",
  type = "button",
  asChild = false,
  ...props
}: ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & { asChild?: boolean }) {
  const Component = asChild ? Slot : "button";
  return (
    <Component
      {...props}
      type={type}
      data-slot="button"
      className={cn(buttonVariants({ variant, size }), className)}
    />
  );
}
export { Button, buttonVariants };
