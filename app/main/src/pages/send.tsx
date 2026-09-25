import * as React from "react"
import {
  ArrowLeftIcon,
  CheckIcon,
  FileIcon,
  PlusIcon,
  RotateCwIcon,
  XIcon,
} from "lucide-react"

import type { EndpointInfo } from "@bindings/EndpointInfo"

import { DeviceIcon } from "@/components/device-icon"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemFooter,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Progress } from "@/components/ui/progress"
import { Separator } from "@/components/ui/separator"
import { Spinner } from "@/components/ui/spinner"
import { type Transfer, isTerminal, useQuickShare } from "@/hooks/quick-share"
import { fileName, percent, plural } from "@/lib/format"
import { api } from "@/lib/tauri"

export function SendPage({
  files,
  onAddFiles,
  onRemoveFile,
  onBack,
}: {
  files: string[]
  onAddFiles: () => void
  onRemoveFile: (path: string) => void
  onBack: () => void
}) {
  const { endpoints, clearEndpoints } = useQuickShare()

  React.useEffect(() => {
    api.startDiscovery()
    return () => {
      api.stopDiscovery()
      clearEndpoints()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="flex min-h-svh flex-col gap-4 p-4">
      <header className="flex items-center gap-2">
        <Button variant="ghost" size="icon" aria-label="Back" onClick={onBack}>
          <ArrowLeftIcon />
        </Button>
        <h1 className="font-heading text-base font-semibold">
          Send {plural(files.length, "file")}
        </h1>
      </header>

      <ItemGroup role="list" className="gap-1">
        {files.map((path) => (
          <Item key={path} size="xs" variant="muted" role="listitem">
            <ItemMedia variant="icon">
              <FileIcon />
            </ItemMedia>
            <ItemContent className="min-w-0">
              <ItemTitle className="w-full truncate">
                {fileName(path)}
              </ItemTitle>
            </ItemContent>
            <ItemActions>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Remove ${fileName(path)}`}
                onClick={() => onRemoveFile(path)}
              >
                <XIcon />
              </Button>
            </ItemActions>
          </Item>
        ))}
      </ItemGroup>
      <Button variant="outline" className="self-start" onClick={onAddFiles}>
        <PlusIcon data-icon="inline-start" />
        Add files
      </Button>

      <Separator />

      <section className="flex flex-1 flex-col gap-2">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-medium text-muted-foreground">
            Nearby devices
          </h2>
          <Spinner className="text-muted-foreground" />
        </div>

        {endpoints.length === 0 ? (
          <Empty className="flex-1">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Spinner />
              </EmptyMedia>
              <EmptyTitle>Looking for nearby devices</EmptyTitle>
              <EmptyDescription>
                On the other device, open Quick Share and set it to be visible
                to everyone. Both devices need Wi-Fi on the same network, and
                Bluetooth helps phones notice you.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <ItemGroup role="list">
            {endpoints.map((endpoint) => (
              <DeviceRow key={endpoint.id} endpoint={endpoint} files={files} />
            ))}
          </ItemGroup>
        )}
      </section>
    </div>
  )
}

function DeviceRow({
  endpoint,
  files,
}: {
  endpoint: EndpointInfo
  files: string[]
}) {
  const { transfers, dismiss } = useQuickShare()
  const [requested, setRequested] = React.useState(false)
  const transfer = requested
    ? transfers.find((t) => t.id === endpoint.id && t.direction === "Outbound")
    : undefined
  const busy = requested && (!transfer || !isTerminal(transfer.state))

  const send = () => {
    if (!endpoint.ip || !endpoint.port || files.length === 0) return
    // Forget the previous attempt so the row shows this one.
    dismiss(endpoint.id)
    setRequested(true)
    api.send({
      id: endpoint.id,
      name: endpoint.name ?? "Unknown device",
      addr: `${endpoint.ip}:${endpoint.port}`,
      ob: { Files: files },
    })
  }

  return (
    <Item variant="outline" role="listitem">
      <ItemMedia variant="icon">
        <DeviceIcon type={endpoint.rtype} />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="w-full truncate">
          {endpoint.name ?? "Unknown device"}
        </ItemTitle>
        <ItemDescription>
          {requested ? sendStatus(transfer) : "Ready"}
        </ItemDescription>
      </ItemContent>
      <ItemActions>
        {busy ? (
          <Button
            variant="outline"
            size="sm"
            disabled={!transfer}
            onClick={() =>
              transfer && api.transferAction(transfer.id, "CancelTransfer")
            }
          >
            Cancel
          </Button>
        ) : transfer?.state === "Finished" ? (
          <CheckIcon className="text-primary" aria-label="Sent" />
        ) : (
          <Button size="sm" disabled={files.length === 0} onClick={send}>
            {transfer ? <RotateCwIcon data-icon="inline-start" /> : null}
            {transfer ? "Retry" : "Send"}
          </Button>
        )}
      </ItemActions>
      {transfer?.state === "SendingFiles" && (
        <ItemFooter>
          <Progress
            className="w-full"
            value={percent(
              transfer.meta?.ack_bytes ?? 0,
              transfer.meta?.total_bytes ?? 0
            )}
          />
        </ItemFooter>
      )}
    </Item>
  )
}

function sendStatus(transfer: Transfer | undefined) {
  switch (transfer?.state) {
    case undefined:
      return "Connecting…"
    case "SentIntroduction":
      return transfer.meta?.pin_code
        ? `Waiting to accept · PIN ${transfer.meta.pin_code}`
        : "Waiting to accept"
    case "SendingFiles":
      return "Sending…"
    case "Finished":
      return "Sent"
    case "Rejected":
      return "Declined"
    case "Cancelled":
      return "Cancelled"
    case "Disconnected":
      return "Couldn't send. Try again."
    default:
      return "Connecting…"
  }
}
