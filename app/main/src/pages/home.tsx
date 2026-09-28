import * as React from "react"
import { ChevronDownIcon, UploadIcon } from "lucide-react"
import { cn } from "cn"

import { Collapse } from "@/components/collapse"
import { TransferItem } from "@/components/transfer-item"
import { Button } from "@/components/ui/button"
import { ItemGroup } from "@/components/ui/item"
import { Switch } from "@/components/ui/switch"
import { isShown, isTerminal, useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

const VISIBILITY_LABEL = {
  Visible: "Visible to everyone",
  Temporarily: "Visible for a minute",
  Invisible: "Hidden",
}

export function HomePage({ onPickFiles }: { onPickFiles: () => void }) {
  const { settings, transfers } = useQuickShare()
  const shown = transfers.filter(isShown)
  // The newest transfer and any still going stay visible; older finished
  // ones fold away.
  const latest = shown.filter((t, i) => i === 0 || !isTerminal(t.state))
  const older = shown.filter((t) => !latest.includes(t))
  const [expanded, setExpanded] = React.useState(false)
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
        <section className="flex flex-col">
          <ItemGroup role="list" className="gap-2">
            {latest.map((t) => (
              <TransferItem key={t.id} transfer={t} />
            ))}
          </ItemGroup>
          {older.length > 0 && (
            <>
              <Collapse open={expanded}>
                <ItemGroup role="list" className="gap-2 pt-2">
                  {older.map((t) => (
                    <TransferItem key={t.id} transfer={t} />
                  ))}
                </ItemGroup>
              </Collapse>
              <Button
                variant="ghost"
                size="sm"
                className="mt-1 self-center text-muted-foreground"
                aria-expanded={expanded}
                onClick={() => setExpanded(!expanded)}
              >
                {expanded ? "Show less" : `Show ${older.length} more`}
                <ChevronDownIcon
                  data-icon="inline-end"
                  className={cn(
                    "transition-transform duration-200",
                    expanded && "rotate-180"
                  )}
                />
              </Button>
            </>
          )}
        </section>
      )}

      <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl bg-muted/60 text-center transition-colors">
        <UploadIcon className="text-muted-foreground" />
        <p className="text-muted-foreground">Drop files to send</p>
        <Button onClick={onPickFiles}>Choose files</Button>
      </div>
    </div>
  )
}
