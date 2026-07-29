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
      "peer inline-flex h-6 w-[42px] shrink-0 cursor-pointer items-center rounded-[7px] border transition-[background-color,border-color] duration-200 ease-out",
      "border-black/20 bg-[#fffefa] hover:border-black/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#157252]/35 focus-visible:ring-offset-2 focus-visible:ring-offset-[#f5f3ee]",
      "data-[state=checked]:border-[#157252] data-[state=checked]:bg-[#157252] data-[state=checked]:hover:border-[#0e513a] data-[state=checked]:hover:bg-[#0e513a]",
      "disabled:cursor-not-allowed disabled:opacity-45",
      "dark:border-white/20 dark:bg-[#252725] dark:hover:border-white/35 dark:focus-visible:ring-[#56d59d]/45 dark:focus-visible:ring-offset-[#151616]",
      "dark:data-[state=checked]:border-[#56d59d] dark:data-[state=checked]:bg-[#56d59d] dark:data-[state=checked]:hover:border-[#a5f1c8] dark:data-[state=checked]:hover:bg-[#a5f1c8]",
      className
    )}
    {...props}
    ref={ref}
  >
    <SwitchPrimitives.Thumb
      className={cn(
        "pointer-events-none block h-4 w-4 rounded-[4px] bg-[#686a64] transition-[transform,background-color] duration-200 ease-out",
        "data-[state=unchecked]:translate-x-[3px] data-[state=checked]:translate-x-[21px] data-[state=checked]:bg-[#fffefa]",
        "dark:bg-[#aaa9a1] dark:data-[state=checked]:bg-[#151616]"
      )}
    />
  </SwitchPrimitives.Root>
))
Switch.displayName = SwitchPrimitives.Root.displayName

export { Switch }
