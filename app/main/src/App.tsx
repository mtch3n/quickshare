import * as React from "react"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { open } from "@tauri-apps/plugin-dialog"
import { openPath } from "@tauri-apps/plugin-opener"
import { UploadIcon } from "lucide-react"

import { IncomingDialog } from "@/components/incoming-dialog"
import { toast } from "@/components/ui/toast"
import { useQuickShare } from "@/hooks/quick-share"
import { api, on } from "@/lib/tauri"
import { describeContent } from "@/lib/transfer"
import { HomePage } from "@/pages/home"
import { SendPage } from "@/pages/send"
import { SettingsPage } from "@/pages/settings"

type Page = "home" | "send" | "settings"

export function App() {
  const [page, setPage] = React.useState<Page>("home")
  const [files, setFiles] = React.useState<string[]>([])
  const [dragging, setDragging] = React.useState(false)

  const addFiles = React.useCallback((paths: string[]) => {
    if (paths.length === 0) return
    setFiles((current) => [...new Set([...current, ...paths])])
    setPage("send")
  }, [])

  const pickFiles = React.useCallback(async () => {
    const picked = await open({ multiple: true, title: "Choose files to send" })
    if (picked) addFiles(picked)
  }, [addFiles])

  React.useEffect(() => {
    // Files from "Send with Quick Share" in the file manager.
    api.takePendingFiles().then(addFiles)

    const unlisten = [
      on("send_files", addFiles),
      on("pick_files", pickFiles),
      getCurrentWebview().onDragDropEvent(({ payload }) => {
        setDragging(payload.type === "enter" || payload.type === "over")
        if (payload.type === "drop") addFiles(payload.paths)
      }),
    ]

    return () => unlisten.forEach((p) => p.then((f) => f()))
  }, [addFiles, pickFiles])

  useReceivedToasts()

  const leaveSend = () => {
    setFiles([])
    setPage("home")
  }

  return (
    <>
      {page === "home" && (
        <HomePage
          onPickFiles={pickFiles}
          onOpenSettings={() => setPage("settings")}
        />
      )}
      {page === "send" && (
        <SendPage
          files={files}
          onAddFiles={pickFiles}
          onRemoveFile={(path) => {
            const rest = files.filter((f) => f !== path)
            if (rest.length === 0) leaveSend()
            else setFiles(rest)
          }}
          onBack={leaveSend}
        />
      )}
      {page === "settings" && <SettingsPage onBack={() => setPage("home")} />}

      <IncomingDialog />

      {dragging && (
        <div className="pointer-events-none fixed inset-2 flex flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed border-primary bg-background/90 text-primary">
          <UploadIcon />
          <p className="font-medium">Drop to send</p>
        </div>
      )}
    </>
  )
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
          ? { children: "Open folder", onClick: () => openPath(folder) }
          : undefined,
      })
    }
  }, [transfers])
}

export default App
