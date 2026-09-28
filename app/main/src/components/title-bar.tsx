import * as React from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { ArrowLeftIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"

/**
 * Replaces the system title bar and doubles as the page header: back button
 * or app icon, the page title, page actions, then close. Drag it anywhere
 * that isn't a button.
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
      className="flex h-12 shrink-0 items-center gap-2 px-2 select-none"
    >
      {onBack ? (
        <Button variant="ghost" size="icon" aria-label="Back" onClick={onBack}>
          <ArrowLeftIcon />
        </Button>
      ) : (
        <img
          data-tauri-drag-region
          src="/icon.svg"
          alt=""
          className="ml-1 size-7"
        />
      )}
      <h1
        data-tauri-drag-region
        className="min-w-0 flex-1 truncate font-heading text-base font-semibold"
      >
        {title}
      </h1>
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
  )
}
