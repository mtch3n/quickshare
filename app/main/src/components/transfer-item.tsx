import { writeText } from "@tauri-apps/plugin-clipboard-manager"
import { openPath, openUrl } from "@tauri-apps/plugin-opener"
import {
  CheckIcon,
  CopyIcon,
  ExternalLinkIcon,
  FolderOpenIcon,
  XIcon,
} from "lucide-react"

import { DeviceIcon } from "@/components/device-icon"
import { Button } from "@/components/ui/button"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemFooter,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Progress } from "@/components/ui/progress"
import { toast } from "@/components/ui/toast"
import { type Transfer, isTerminal, useQuickShare } from "@/hooks/quick-share"
import { percent } from "@/lib/format"
import { describeContent } from "@/lib/transfer"
import { api } from "@/lib/tauri"

function status(transfer: Transfer) {
  const what = describeContent(transfer)
  const inbound = transfer.direction === "Inbound"

  switch (transfer.state) {
    case "WaitingForUserConsent":
      return `Wants to share ${what}`
    case "SentIntroduction":
      return transfer.meta?.pin_code
        ? `Waiting to accept · PIN ${transfer.meta.pin_code}`
        : "Waiting to accept"
    case "ReceivingFiles":
      return `Receiving ${what}`
    case "SendingFiles":
      return `Sending ${what}`
    case "Finished":
      return inbound ? `Received ${what}` : `Sent ${what}`
    case "Rejected":
      return inbound ? "You declined" : "Declined"
    case "Cancelled":
      return "Cancelled"
    case "Disconnected":
      return "Connection lost"
    default:
      return "Connecting…"
  }
}

async function copy(text: string) {
  try {
    await writeText(text)
    toast.add({ title: "Copied to clipboard", type: "success" })
  } catch (e) {
    toast.add({ title: "Couldn't copy", description: String(e), type: "error" })
  }
}

export function TransferItem({ transfer }: { transfer: Transfer }) {
  const { dismiss } = useQuickShare()
  const { id, state, meta, direction } = transfer
  const active = state === "ReceivingFiles" || state === "SendingFiles"
  const text = meta?.text_payload

  return (
    <Item variant="outline" size="sm" role="listitem">
      <ItemMedia variant="icon">
        {state === "Finished" ? (
          <CheckIcon />
        ) : (
          <DeviceIcon type={meta?.source?.device_type} />
        )}
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="w-full truncate">
          {meta?.source?.name ?? "Unknown device"}
        </ItemTitle>
        <ItemDescription className="truncate">
          {status(transfer)}
        </ItemDescription>
      </ItemContent>
      <ItemActions>
        {state === "WaitingForUserConsent" && (
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() => api.transferAction(id, "RejectTransfer")}
            >
              Decline
            </Button>
            <Button
              size="sm"
              onClick={() => api.transferAction(id, "AcceptTransfer")}
            >
              Accept
            </Button>
          </>
        )}
        {(active || state === "SentIntroduction") && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => api.transferAction(id, "CancelTransfer")}
          >
            Cancel
          </Button>
        )}
        {state === "Finished" && direction === "Inbound" && text && (
          <Button variant="outline" size="sm" onClick={() => copy(text)}>
            <CopyIcon data-icon="inline-start" />
            Copy
          </Button>
        )}
        {state === "Finished" && meta?.text_type === "Url" && text && (
          <Button size="sm" onClick={() => openUrl(text)}>
            <ExternalLinkIcon data-icon="inline-start" />
            Open
          </Button>
        )}
        {state === "Finished" &&
          direction === "Inbound" &&
          !text &&
          meta?.destination && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => openPath(meta.destination!)}
            >
              <FolderOpenIcon data-icon="inline-start" />
              Open folder
            </Button>
          )}
        {isTerminal(state) && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Dismiss"
            onClick={() => dismiss(id)}
          >
            <XIcon />
          </Button>
        )}
      </ItemActions>
      {active && (
        <ItemFooter>
          <Progress
            className="w-full"
            value={percent(meta?.ack_bytes ?? 0, meta?.total_bytes ?? 0)}
          />
        </ItemFooter>
      )}
      {state === "Finished" && text && meta?.text_type !== "Url" && (
        <ItemFooter>
          <p className="line-clamp-3 w-full text-sm break-words text-muted-foreground select-text">
            {text}
          </p>
        </ItemFooter>
      )}
    </Item>
  )
}
