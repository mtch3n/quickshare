import { HistoryIcon, SettingsIcon, UploadIcon } from "lucide-react"

import { TransferItem } from "@/components/transfer-item"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { ItemGroup } from "@/components/ui/item"
import { Switch } from "@/components/ui/switch"
import { isShown, isTerminal, useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

export function HomePage({
  onPickFiles,
  onOpenSettings,
}: {
  onPickFiles: () => void
  onOpenSettings: () => void
}) {
  const { settings, transfers, clearFinished } = useQuickShare()
  const shown = transfers.filter(isShown)

  return (
    <div className="flex min-h-svh flex-col gap-4 p-4">
      <header className="flex items-center gap-3">
        <img src="/icon.svg" alt="" className="size-9" />
        <div className="flex min-w-0 flex-1 flex-col">
          <h1 className="font-heading text-base font-semibold">RQuickShare</h1>
          <p className="truncate text-sm text-muted-foreground">
            {settings?.deviceName}
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Settings"
          onClick={onOpenSettings}
        >
          <SettingsIcon />
        </Button>
      </header>

      <VisibilityCard />

      <Card className="border-dashed">
        <CardHeader className="items-center text-center">
          <UploadIcon className="mx-auto text-muted-foreground" />
          <CardTitle>Send files</CardTitle>
          <CardDescription>
            Drop files anywhere in this window, or choose them.
          </CardDescription>
          <Button className="mx-auto mt-2" onClick={onPickFiles}>
            Choose files
          </Button>
        </CardHeader>
      </Card>

      <section className="flex flex-1 flex-col gap-2">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-medium text-muted-foreground">
            Activity
          </h2>
          {shown.some((t) => isTerminal(t.state)) && (
            <Button variant="ghost" size="xs" onClick={clearFinished}>
              Clear
            </Button>
          )}
        </div>

        {shown.length === 0 ? (
          <Empty className="flex-1">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <HistoryIcon />
              </EmptyMedia>
              <EmptyTitle>Nothing here yet</EmptyTitle>
              <EmptyDescription>
                Files you send and receive show up here.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <ItemGroup role="list">
            {shown.map((t) => (
              <TransferItem key={t.id} transfer={t} />
            ))}
          </ItemGroup>
        )}
      </section>
    </div>
  )
}

function VisibilityCard() {
  const { settings } = useQuickShare()
  const visibility = settings?.visibility

  const description = {
    Visible: "Nearby devices can send you files. You're always asked first.",
    Temporarily: "Visible to everyone for a minute.",
    Invisible:
      "Nobody can find you. You'll be notified when someone nearby is sharing.",
  }[visibility ?? "Visible"]

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>
          {visibility === "Invisible" ? "Hidden" : "Visible to everyone"}
        </CardTitle>
        <CardDescription>{description}</CardDescription>
        <CardAction>
          <Switch
            aria-label="Visible to everyone"
            checked={visibility !== undefined && visibility !== "Invisible"}
            disabled={!settings}
            onCheckedChange={(checked) =>
              api.setVisibility(checked ? "Visible" : "Invisible")
            }
          />
        </CardAction>
      </CardHeader>
    </Card>
  )
}
