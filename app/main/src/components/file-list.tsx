import * as React from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
import {
  ChevronDownIcon,
  FileArchiveIcon,
  FileAudioIcon,
  FileIcon,
  FileTextIcon,
  FileVideoIcon,
  FolderIcon,
  ImageIcon,
  XIcon,
} from "lucide-react"
import { cn } from "cn"

import { Collapse } from "@/components/collapse"
import { Button } from "@/components/ui/button"
import {
  type FileKind,
  fileKind,
  fileName,
  formatBytes,
  plural,
} from "@/lib/format"
import { type FileSummary, api } from "@/lib/tauri"

const KIND_ICONS: Record<FileKind, typeof FileIcon> = {
  image: ImageIcon,
  video: FileVideoIcon,
  audio: FileAudioIcon,
  archive: FileArchiveIcon,
  text: FileTextIcon,
  other: FileIcon,
}

/** Sizes of `paths`, which also clears them for image previews. */
function useSummaries(paths: string[]) {
  const [summaries, setSummaries] = React.useState<Map<string, FileSummary>>(
    () => new Map()
  )

  React.useEffect(() => {
    let current = true
    api.inspectFiles(paths).then((list) => {
      if (current) setSummaries(new Map(list.map((s) => [s.path, s])))
    })
    return () => {
      current = false
    }
  }, [paths])

  return summaries
}

/** The files about to be sent: one previewed, or several behind a toggle. */
export function FileList({
  files,
  onRemove,
}: {
  files: string[]
  onRemove: (path: string) => void
}) {
  const summaries = useSummaries(files)
  const [open, setOpen] = React.useState(false)

  if (files.length === 1) {
    return (
      <SingleFile
        path={files[0]}
        summary={summaries.get(files[0])}
        onRemove={onRemove}
      />
    )
  }

  const known = files.every((f) => summaries.has(f))
  const total = files.reduce((sum, f) => sum + (summaries.get(f)?.size ?? 0), 0)

  return (
    <div className="flex flex-col rounded-lg bg-muted/50">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-3 rounded-lg px-2.5 py-2 text-left outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
      >
        <div className="flex -space-x-4">
          {files.slice(0, 3).map((path) => (
            <Thumbnail
              key={path}
              path={path}
              summary={summaries.get(path)}
              className="size-10 ring-2 ring-muted"
            />
          ))}
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="text-sm font-medium">
            {plural(files.length, "file")}
          </span>
          <span className="text-xs text-muted-foreground">
            {known ? formatBytes(total) : " "}
          </span>
        </div>
        <ChevronDownIcon
          className={cn(
            "size-4 text-muted-foreground transition-transform duration-200",
            open && "rotate-180"
          )}
        />
      </button>

      <Collapse open={open}>
        <ul>
          {files.map((path) => (
            <li
              key={path}
              className="flex items-center gap-2.5 px-2.5 py-1.5 last:pb-2.5"
            >
              <Thumbnail
                path={path}
                summary={summaries.get(path)}
                className="size-8"
              />
              <FileText path={path} summary={summaries.get(path)} />
              <RemoveButton path={path} onRemove={onRemove} />
            </li>
          ))}
        </ul>
      </Collapse>
    </div>
  )
}

function SingleFile({
  path,
  summary,
  onRemove,
}: {
  path: string
  summary: FileSummary | undefined
  onRemove: (path: string) => void
}) {
  return (
    <div className="flex items-center gap-3 rounded-lg bg-muted/50 px-2.5 py-2">
      <Thumbnail path={path} summary={summary} className="size-12" />
      <FileText path={path} summary={summary} />
      <RemoveButton path={path} onRemove={onRemove} />
    </div>
  )
}

/** An image's own thumbnail, or an icon for its kind. */
function Thumbnail({
  path,
  summary,
  className,
}: {
  path: string
  summary: FileSummary | undefined
  className?: string
}) {
  const [failed, setFailed] = React.useState(false)
  const kind = fileKind(path)
  const Icon = summary?.isDir ? FolderIcon : KIND_ICONS[kind]
  const box = cn(
    "flex shrink-0 items-center justify-center overflow-hidden rounded-md bg-background text-muted-foreground",
    className
  )

  if (kind === "image" && summary && !summary.isDir && !failed) {
    return (
      <img
        src={convertFileSrc(path)}
        alt=""
        loading="lazy"
        decoding="async"
        onError={() => setFailed(true)}
        className={cn(box, "object-cover")}
      />
    )
  }
  return (
    <div className={box}>
      <Icon className="size-4" />
    </div>
  )
}

function FileText({
  path,
  summary,
}: {
  path: string
  summary: FileSummary | undefined
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <span className="truncate text-sm" title={path}>
        {fileName(path)}
      </span>
      <span className="text-xs text-muted-foreground">
        {summary ? formatBytes(summary.size) : " "}
      </span>
    </div>
  )
}

function RemoveButton({
  path,
  onRemove,
}: {
  path: string
  onRemove: (path: string) => void
}) {
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      aria-label={`Remove ${fileName(path)}`}
      onClick={() => onRemove(path)}
    >
      <XIcon />
    </Button>
  )
}
