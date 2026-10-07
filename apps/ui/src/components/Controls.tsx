import type { ComponentProps } from "react";
import { buttonVariants } from "./ui/button.js";
import { Input } from "./ui/input.js";

const variants = {
  primary: "default",
  secondary: "outline",
  tertiary: "ghost",
  danger: "destructive",
} as const;

/** Presentation only. Hosts decide whether an action is available. */
export function Button({
  variant = "secondary",
  type = "button",
  className,
  ...props
}: ComponentProps<"button"> & {
  variant?: keyof typeof variants;
}) {
  return (
    <button
      {...props}
      type={type}
      data-slot="button"
      className={buttonVariants({ variant: variants[variant], className })}
    />
  );
}
export const TextInput = Input;
