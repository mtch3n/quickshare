import * as React from "react"
import { SendIcon, SettingsIcon, UploadIcon } from "lucide-react"

import { TransferItem } from "@/components/transfer-item"
import { Button } from "@/components/ui/button"
import { ItemGroup } from "@/components/ui/item"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { isShown, useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

const VISIBILITY_LABEL = {
  Visible: "Visible to everyone",
  Temporarily: "Visible for a minute",
  Invisible: "Hidden",
}

export function HomePage({
  onPickFiles,
  onSendText,
  onOpenSettings,
}: {
  onPickFiles: () => void
  onSendText: (text: string) => void
  onOpenSettings: () => void
}) {
  const { settings, transfers } = useQuickShare()
  const shown = transfers.filter(isShown)
  const visibility = settings?.visibility
  const [text, setText] = React.useState("")

  const handleSendText = () => {
    if (text.trim()) {
      onSendText(text)
      setText("")
    }
  }

  return (
    <div className="flex h-svh flex-col gap-4 p-4">
      <header className="flex items-center gap-3">
        <img src="/icon.svg" alt="" className="size-9" />
        <div className="flex min-w-0 flex-1 flex-col">
          <h1 className="truncate font-heading text-base font-semibold">
            {settings?.deviceName}
          </h1>
          <p className="text-sm text-muted-foreground">
            {VISIBILITY_LABEL[visibility ?? "Visible"]}
          </p>
        </div>
        <Switch
          aria-label="Visible to everyone"
          checked={visibility !== undefined && visibility !== "Invisible"}
          disabled={!settings}
          onCheckedChange={(checked) =>
            api.setVisibility(checked ? "Visible" : "Invisible")
          }
        />
        <Button
          variant="ghost"
          size="icon"
          aria-label="Settings"
          onClick={onOpenSettings}
        >
          <SettingsIcon />
        </Button>
      </header>

      {shown.length > 0 && (
        <ItemGroup role="list">
          {shown.map((t) => (
            <TransferItem key={t.id} transfer={t} />
          ))}
        </ItemGroup>
      )}

      <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border-2 border-dashed text-center">
        <UploadIcon className="text-muted-foreground" />
        <p className="text-muted-foreground">Drop files to send</p>
        <Button variant="outline" onClick={onPickFiles}>
          Choose files
        </Button>
      </div>

      <div className="relative">
        <Textarea
          aria-label="Text or link to send"
          placeholder="Send text or a link…"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault()
              handleSendText()
            }
          }}
          className="max-h-32 min-h-11 resize-none pr-12"
        />
        <Button
          size="icon-sm"
          aria-label="Send"
          onClick={handleSendText}
          disabled={!text.trim()}
          className="absolute right-2 bottom-2"
        >
          <SendIcon />
        </Button>
      </div>
    </div>
  )
}
