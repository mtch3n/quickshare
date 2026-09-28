import * as React from "react"
import { DeviceIcon } from "@/components/device-icon"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Checkbox } from "@/components/ui/checkbox"
import { useQuickShare } from "@/hooks/quick-share"
import { fileName, formatBytes } from "@/lib/format"
import { api } from "@/lib/tauri"
import { describeContent } from "@/lib/transfer"

const MAX_LISTED_FILES = 5

/** Asks whether to accept the oldest pending incoming transfer. */
export function IncomingDialog() {
  const { transfers } = useQuickShare()
  const [trust, setTrust] = React.useState(false)
  const pending = transfers.findLast(
    (t) => t.direction === "Inbound" && t.state === "WaitingForUserConsent"
  )

  if (!pending) return null

  const meta = pending.meta
  const files = meta?.files ?? []
  const name = meta?.source?.name ?? "A nearby device"
  // LocalSend senders can only be trusted by the certificate they presented;
  // one without (plain HTTP) can't be recognised later.
  const localsend = pending.id.startsWith("ls-")
  const fingerprint = meta?.source?.fingerprint ?? null
  const trustable = !localsend || fingerprint !== null
  const respond = async (accept: boolean) => {
    if (accept && trust && trustable) {
      await api.trustDevice({
        name,
        fingerprint: localsend ? fingerprint : null,
      })
    }
    api.transferAction(pending.id, accept ? "AcceptTransfer" : "RejectTransfer")
  }

  return (
    <AlertDialog open>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogMedia>
            <DeviceIcon type={meta?.source?.device_type} />
          </AlertDialogMedia>
          <AlertDialogTitle>{name} wants to share</AlertDialogTitle>
          <AlertDialogDescription>
            {describeContent(pending)}
            {files.length > 0 && meta?.total_bytes
              ? ` · ${formatBytes(meta.total_bytes)}`
              : ""}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {files.length > 1 && (
          <ul className="flex flex-col gap-1 text-sm">
            {files.slice(0, MAX_LISTED_FILES).map((f) => (
              <li key={f} className="truncate">
                {fileName(f)}
              </li>
            ))}
            {files.length > MAX_LISTED_FILES && (
              <li className="text-muted-foreground">
                and {files.length - MAX_LISTED_FILES} more
              </li>
            )}
          </ul>
        )}

        {meta?.text_description && (
          <p className="line-clamp-4 text-sm break-words">
            {meta.text_description}
          </p>
        )}

        {meta?.pin_code && (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            Check that the other device shows
            <Badge variant="secondary">{meta.pin_code}</Badge>
          </p>
        )}

        {trustable && (
          <div className="flex items-center gap-2">
            <Checkbox
              id="trust-device"
              checked={trust}
              onCheckedChange={setTrust}
            />
            <label
              htmlFor="trust-device"
              className="cursor-pointer text-sm leading-none font-medium peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
            >
              Always accept from {name}
            </label>
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel onClick={() => respond(false)}>
            Decline
          </AlertDialogCancel>
          <AlertDialogAction onClick={() => respond(true)}>
            Accept
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
