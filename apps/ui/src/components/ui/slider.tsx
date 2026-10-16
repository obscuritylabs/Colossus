// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
import type { ComponentProps } from "react";
import * as SliderPrimitive from "@radix-ui/react-slider";
import { cn } from "../../lib/utils.js";

/** A single-value, keyboard-accessible slider using Colossus tokens. */
export function Slider({
  className,
  "aria-label": label,
  "aria-valuetext": valueText,
  ref,
  ...props
}: ComponentProps<typeof SliderPrimitive.Root>) {
  return (
    <SliderPrimitive.Root
      {...props}
      data-slot="slider"
      className={cn("ui-slider", className)}
    >
      <SliderPrimitive.Track className="ui-slider-track">
        <SliderPrimitive.Range className="ui-slider-range" />
      </SliderPrimitive.Track>
      <SliderPrimitive.Thumb
        ref={ref}
        aria-label={label}
        aria-valuetext={valueText}
        className="ui-slider-thumb"
      />
    </SliderPrimitive.Root>
  );
}
