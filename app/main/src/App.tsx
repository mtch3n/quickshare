import * as React from "react"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { open } from "@tauri-apps/plugin-dialog"
import { SettingsIcon, UploadIcon } from "lucide-react"

import type { OutboundPayload } from "@bindings/OutboundPayload"

import { IncomingDialog } from "@/components/incoming-dialog"
import { Button } from "@/components/ui/button"
import { TitleBar } from "@/components/title-bar"
import { toast } from "@/components/ui/toast"
import { useQuickShare } from "@/hooks/quick-share"
import { api, on } from "@/lib/tauri"
import { plural } from "@/lib/format"
import { describeContent } from "@/lib/transfer"
import { HomePage } from "@/pages/home"
import { SendPage } from "@/pages/send"
import { SettingsPage } from "@/pages/settings"

type Page = "home" | "send" | "settings"

export function App() {
  const [page, setPage] = React.useState<Page>("home")
  const [payload, setPayload] = React.useState<OutboundPayload | null>(null)
  const [dragging, setDragging] = React.useState(false)

  const startSending = React.useCallback((newPayload: OutboundPayload) => {
    setPayload(newPayload)
    setPage("send")
  }, [])

  /** Adds to the files already being sent, or starts sending these. */
  const addFiles = React.useCallback((paths: string[]) => {
    if (paths.length === 0) return
    setPayload((current) =>
      current && "Files" in current
        ? { Files: [...new Set([...current.Files, ...paths])] }
        : { Files: paths }
    )
    setPage("send")
  }, [])

  const sendText = React.useCallback(
    (text: string) => {
      if (text.length === 0) return
      startSending({ Text: text })
    },
    [startSending]
  )

  const pickFiles = React.useCallback(async () => {
    const picked = await open({ multiple: true, title: "Choose files to send" })
    if (picked) addFiles(picked)
  }, [addFiles])

  React.useEffect(() => {
    // Files from "Send with Quick Share" in the file manager.
    api.takePendingFiles().then(addFiles)

    const unlisten = [
      on("send_files", addFiles),
      on("send_text", sendText),
      on("pick_files", pickFiles),
      getCurrentWebview().onDragDropEvent(({ payload }) => {
        setDragging(payload.type === "enter" || payload.type === "over")
        if (payload.type === "drop") addFiles(payload.paths)
      }),
    ]

    return () => unlisten.forEach((p) => p.then((f) => f()))
  }, [addFiles, sendText, pickFiles])

  useReceivedToasts()

  const leaveSend = () => {
    setPayload(null)
    setPage("home")
  }

  return (
    <div className="flex h-svh flex-col">
      <TitleBar
        {...(page === "home"
          ? {
              title: "QuickShare",
              actions: (
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label="Settings"
                  onClick={() => setPage("settings")}
                >
                  <SettingsIcon />
                </Button>
              ),
            }
          : page === "send"
            ? { title: sendTitle(payload), onBack: leaveSend }
            : { title: "Settings", onBack: () => setPage("home") })}
      />
      <main
        key={page}
        className="min-h-0 flex-1 animate-in overflow-y-auto duration-200 fade-in-0"
      >
        {page === "home" && <HomePage onPickFiles={pickFiles} />}
        {page === "send" && payload && (
          <SendPage
            payload={payload}
            onAddFiles={pickFiles}
            onRemoveFile={(path) => {
              if ("Files" in payload) {
                const rest = payload.Files.filter((f) => f !== path)
                if (rest.length === 0) leaveSend()
                else setPayload({ Files: rest })
              }
            }}
          />
        )}
        {page === "settings" && <SettingsPage />}
      </main>

      <IncomingDialog />

      {dragging && (
        <div className="pointer-events-none fixed inset-2 flex animate-in flex-col items-center justify-center gap-2 rounded-xl bg-background/95 text-primary ring-2 ring-primary/40 duration-150 fade-in-0 zoom-in-95">
          <UploadIcon />
          <p className="font-medium">Drop to send</p>
        </div>
      )}
    </div>
  )
}

function sendTitle(payload: OutboundPayload | null) {
  if (!payload || "Text" in payload) return "Send text"
  return `Send ${plural(payload.Files.length, "file")}`
}

/** Toasts once per finished incoming transfer. */
function useReceivedToasts() {
  const { transfers } = useQuickShare()
  const announced = React.useRef(new Set<string>())

  React.useEffect(() => {
    for (const t of transfers) {
      const key = `${t.id}:${t.state}`
      if (
        t.direction !== "Inbound" ||
        t.state !== "Finished" ||
        announced.current.has(key)
      ) {
        continue
      }
      announced.current.add(key)

      const folder = t.meta?.text_type ? null : t.meta?.destination
      toast.add({
        title: `Received ${describeContent(t)}`,
        description: `From ${t.meta?.source?.name ?? "a nearby device"}`,
        type: "success",
        actionProps: folder
          ? {
              children: "Open folder",
              onClick: () =>
                api.openPath(folder).catch((e) =>
                  toast.add({
                    title: "Couldn't open the folder",
                    description: String(e),
                    type: "error",
                  })
                ),
            }
          : undefined,
      })
    }
  }, [transfers])
}

export default App
