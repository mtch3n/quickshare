import { UploadIcon } from "lucide-react"

import { TransferItem } from "@/components/transfer-item"
import { Button } from "@/components/ui/button"
import { ItemGroup } from "@/components/ui/item"
import { Switch } from "@/components/ui/switch"
import { isShown, useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

const VISIBILITY_LABEL = {
  Visible: "Visible to everyone",
  Temporarily: "Visible for a minute",
  Invisible: "Hidden",
}

export function HomePage({ onPickFiles }: { onPickFiles: () => void }) {
  const { settings, transfers } = useQuickShare()
  const shown = transfers.filter(isShown)
  const visibility = settings?.visibility

  return (
    <div className="flex h-full flex-col gap-4 px-4 pb-4">
      <header className="flex items-center gap-3 px-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <p className="truncate font-medium">{settings?.deviceName}</p>
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
    </div>
  )
}
