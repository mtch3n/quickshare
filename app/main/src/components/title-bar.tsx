import * as React from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { ArrowLeftIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"

/**
 * Replaces the system title bar and doubles as the page header: an optional
 * back button, the page title in the middle, then page actions and close.
 * Drag it anywhere that isn't a button.
 */
export function TitleBar({
  title,
  onBack,
  actions,
}: {
  title: string
  onBack?: () => void
  actions?: React.ReactNode
}) {
  return (
    <div
      data-tauri-drag-region
      className="grid h-12 shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-2 px-2 select-none"
    >
      <div data-tauri-drag-region className="flex">
        {onBack && (
          <Button
            variant="ghost"
            size="icon"
            aria-label="Back"
            onClick={onBack}
          >
            <ArrowLeftIcon />
          </Button>
        )}
      </div>
      <h1
        data-tauri-drag-region
        className="truncate font-heading text-base font-semibold"
      >
        {title}
      </h1>
      <div data-tauri-drag-region className="flex justify-end gap-1">
        {actions}
        <Button
          variant="ghost"
          size="icon"
          aria-label="Close"
          onClick={() => getCurrentWindow().close()}
        >
          <XIcon />
        </Button>
      </div>
    </div>
  )
}
