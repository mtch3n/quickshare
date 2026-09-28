import { cn } from "cn"

/** Shows or hides `children`, animating the height. */
export function Collapse({
  open,
  className,
  children,
}: {
  open: boolean
  className?: string
  children: React.ReactNode
}) {
  return (
    <div
      className={cn(
        "grid transition-[grid-template-rows] duration-200 ease-out",
        open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"
      )}
    >
      <div className={cn("min-h-0 overflow-hidden", className)} inert={!open}>
        {children}
      </div>
    </div>
  )
}
