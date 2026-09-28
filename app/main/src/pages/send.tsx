import * as React from "react"
import {
  CheckIcon,
  FileIcon,
  LinkIcon,
  PlusIcon,
  RotateCwIcon,
} from "lucide-react"

import type { EndpointInfo } from "@bindings/EndpointInfo"
import type { OutboundPayload } from "@bindings/OutboundPayload"

import { DeviceIcon } from "@/components/device-icon"
import { FileList } from "@/components/file-list"
import { Badge } from "@/components/ui/badge"
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
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Progress } from "@/components/ui/progress"
import { Spinner } from "@/components/ui/spinner"
import { type Transfer, isTerminal, useQuickShare } from "@/hooks/quick-share"
import { percent } from "@/lib/format"
import { api } from "@/lib/tauri"

export function SendPage({
  payload,
  onAddFiles,
  onRemoveFile,
}: {
  payload: OutboundPayload
  onAddFiles: () => void
  onRemoveFile: (path: string) => void
}) {
  const { endpoints, clearEndpoints } = useQuickShare()
  const isText = "Text" in payload
  const files = isText ? [] : payload.Files

  React.useEffect(() => {
    api.startDiscovery()
    return () => {
      api.stopDiscovery()
      clearEndpoints()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="flex min-h-full flex-col gap-4 px-4 pb-4">
      {isText ? (
        <Item size="sm" variant="muted" role="listitem">
          <ItemMedia variant="icon">
            {payload.Text.startsWith("http://") ||
            payload.Text.startsWith("https://") ? (
              <LinkIcon />
            ) : (
              <FileIcon />
            )}
          </ItemMedia>
          <ItemContent className="min-w-0">
            <ItemTitle className="line-clamp-2 w-full">
              {payload.Text.split("\n")[0]}
            </ItemTitle>
          </ItemContent>
        </Item>
      ) : (
        <>
          <FileList files={files} onRemove={onRemoveFile} />
          <Button
            variant="secondary"
            className="self-start"
            onClick={onAddFiles}
          >
            <PlusIcon data-icon="inline-start" />
            Add files
          </Button>
        </>
      )}

      <section className="flex flex-1 flex-col gap-2">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-medium text-muted-foreground">
            Nearby devices
          </h2>
          <Spinner className="text-muted-foreground" />
        </div>

        {endpoints.length === 0 ? (
          <Empty className="flex-1 animate-in duration-300 fade-in-0">
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
          <ItemGroup role="list" className="gap-2">
            {endpoints.map((endpoint) => (
              <DeviceRow
                key={endpoint.id}
                endpoint={endpoint}
                payload={payload}
              />
            ))}
          </ItemGroup>
        )}
      </section>
    </div>
  )
}

function DeviceRow({
  endpoint,
  payload,
}: {
  endpoint: EndpointInfo
  payload: OutboundPayload
}) {
  const { transfers, dismiss } = useQuickShare()
  // What was last sent here. Once the files change, a finished attempt no
  // longer describes them, so the row offers to send again.
  const [sent, setSent] = React.useState<OutboundPayload | null>(null)
  const transfer = sent
    ? transfers.find((t) => t.id === endpoint.id && t.direction === "Outbound")
    : undefined
  const busy = sent !== null && (!transfer || !isTerminal(transfer.state))
  const requested = busy || (sent !== null && sent === payload)
  const shown = requested ? transfer : undefined

  const canSend =
    "Files" in payload ? payload.Files.length > 0 : payload.Text.length > 0

  // The PIN of the last attempt, so a second request for one means it was wrong.
  const [pinSent, setPinSent] = React.useState<string | null>(null)
  const [pin, setPin] = React.useState("")
  const needsPin = shown?.state === "PinRequired"

  const send = (withPin: string | null = null) => {
    if (!endpoint.ip || !endpoint.port || !canSend) return
    // Forget the previous attempt so the row shows this one.
    dismiss(endpoint.id)
    setSent(payload)
    setPinSent(withPin)
    api.send({
      id: endpoint.id,
      name: endpoint.name ?? "Unknown device",
      addr: `${endpoint.ip}:${endpoint.port}`,
      protocol: endpoint.protocol,
      ob: payload,
      pin: withPin,
    })
  }

  const sendPin = () => {
    if (pin.trim()) send(pin.trim())
  }

  return (
    <Item
      variant="muted"
      role="listitem"
      className="animate-in duration-200 fade-in-0 slide-in-from-bottom-1"
    >
      <ItemMedia variant="icon">
        <DeviceIcon type={endpoint.rtype} />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="w-full min-w-0">
          <span className="truncate">{endpoint.name ?? "Unknown device"}</span>
          <Badge variant="secondary" className="bg-background/60">
            {endpoint.protocol === "QuickShare" ? "Quick Share" : "LocalSend"}
          </Badge>
        </ItemTitle>
        {requested && (
          <ItemDescription>
            {needsPin
              ? pinSent
                ? "Wrong PIN. Try again."
                : "This device needs its LocalSend PIN"
              : sendStatus(shown)}
          </ItemDescription>
        )}
      </ItemContent>
      <ItemActions>
        {busy ? (
          <Button
            variant="secondary"
            size="sm"
            disabled={!transfer}
            onClick={() =>
              transfer && api.transferAction(transfer.id, "CancelTransfer")
            }
          >
            Cancel
          </Button>
        ) : needsPin ? null : shown?.state === "Finished" ? (
          <CheckIcon
            className="animate-in text-primary duration-300 zoom-in-50"
            aria-label="Sent"
          />
        ) : (
          <Button size="sm" disabled={!canSend} onClick={() => send()}>
            {shown ? <RotateCwIcon data-icon="inline-start" /> : null}
            {shown ? "Retry" : "Send"}
          </Button>
        )}
      </ItemActions>
      {needsPin && (
        <ItemFooter className="animate-in duration-200 fade-in-0 slide-in-from-top-1">
          <form
            className="w-full"
            onSubmit={(e) => {
              e.preventDefault()
              sendPin()
            }}
          >
            <InputGroup>
              <InputGroupInput
                autoFocus
                aria-label="PIN"
                placeholder="PIN"
                inputMode="numeric"
                autoComplete="off"
                value={pin}
                onChange={(e) => setPin(e.target.value)}
                aria-invalid={pinSent !== null}
              />
              <InputGroupAddon align="inline-end">
                <InputGroupButton
                  type="submit"
                  variant="default"
                  size="xs"
                  disabled={!pin.trim()}
                >
                  Send
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
          </form>
        </ItemFooter>
      )}
      {shown?.state === "SendingFiles" && (
        <ItemFooter>
          <Progress
            className="w-full"
            value={percent(
              shown.meta?.ack_bytes ?? 0,
              shown.meta?.total_bytes ?? 0
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
      return transfer.meta?.reason ?? "Couldn't send. Try again."
    default:
      return "Connecting…"
  }
}
