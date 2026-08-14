"use client"

import * as React from "react"
import * as SwitchPrimitives from "@radix-ui/react-switch"

import { cn } from "@/lib/utils"

const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitives.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root>
>(({ className, ...props }, ref) => (
  <SwitchPrimitives.Root
    className={cn(
      "peer inline-flex h-6 w-[42px] shrink-0 cursor-pointer items-center rounded-strip border transition-[background-color,border-color] duration-200 ease-out",
      "border-line-2c bg-paper hover:border-ink-3/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35 focus-visible:ring-offset-2 focus-visible:ring-offset-ground",
      "data-[state=checked]:border-primary data-[state=checked]:bg-primary data-[state=checked]:hover:border-primary-hover data-[state=checked]:hover:bg-primary-hover",
      "disabled:cursor-not-allowed disabled:opacity-45",
      className
    )}
    {...props}
    ref={ref}
  >
    <SwitchPrimitives.Thumb
      className={cn(
        "pointer-events-none block h-4 w-4 rounded-[4px] bg-muted-foreground transition-[transform,background-color] duration-200 ease-out",
        "data-[state=unchecked]:translate-x-[3px] data-[state=checked]:translate-x-[21px] data-[state=checked]:bg-paper"
      )}
    />
  </SwitchPrimitives.Root>
))
Switch.displayName = SwitchPrimitives.Root.displayName

export { Switch }
