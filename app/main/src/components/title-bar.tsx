import { getCurrentWindow } from "@tauri-apps/api/window"
import { XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"

/** Replaces the system title bar: drag anywhere on it, close on the right. */
export function TitleBar() {
  return (
    <div
      data-tauri-drag-region
      className="flex h-10 shrink-0 items-center justify-end px-2 select-none"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Close"
        onClick={() => getCurrentWindow().close()}
      >
        <XIcon />
      </Button>
    </div>
  )
}
