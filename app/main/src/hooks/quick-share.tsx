/* eslint-disable react-refresh/only-export-components */
import * as React from "react"

import type { ChannelMessage } from "@bindings/ChannelMessage"
import type { EndpointInfo } from "@bindings/EndpointInfo"
import type { State } from "@bindings/State"
import type { TransferMetadata } from "@bindings/TransferMetadata"
import type { TransferType } from "@bindings/TransferType"

import { api, on, type Settings } from "@/lib/tauri"

export type Transfer = {
  id: string
  direction: TransferType
  state: State
  meta: TransferMetadata | null
}

const TERMINAL_STATES: State[] = [
  "Finished",
  "Cancelled",
  "Rejected",
  "Disconnected",
]

/** States worth showing; the rest are handshake internals. */
const SHOWN_STATES: State[] = [
  "WaitingForUserConsent",
  "ReceivingFiles",
  "SentIntroduction",
  "SendingFiles",
  ...TERMINAL_STATES,
]

export function isTerminal(state: State) {
  return TERMINAL_STATES.includes(state)
}

export function isShown(transfer: Transfer) {
  return SHOWN_STATES.includes(transfer.state)
}

/** Merges a backend update into the list, newest transfer first. */
function applyMessage(transfers: Transfer[], cm: ChannelMessage): Transfer[] {
  const prev = transfers.find((t) => t.id === cm.id)
  // A new transfer can reuse the id (ip:port) of a finished one.
  const fresh =
    !prev ||
    (isTerminal(prev.state) && cm.state !== null && !isTerminal(cm.state))

  const state = cm.state ?? prev?.state
  if (!state) return transfers

  const next: Transfer = {
    id: cm.id,
    direction: cm.rtype ?? (fresh ? "Inbound" : prev!.direction),
    state,
    meta: cm.meta ?? (fresh ? null : prev!.meta),
  }

  return fresh
    ? [next, ...transfers.filter((t) => t.id !== cm.id)]
    : transfers.map((t) => (t.id === cm.id ? next : t))
}

type QuickShare = {
  settings: Settings | null
  updateSettings: (patch: Partial<Settings>) => void
  transfers: Transfer[]
  endpoints: EndpointInfo[]
  clearEndpoints: () => void
  dismiss: (id: string) => void
  clearFinished: () => void
}

const QuickShareContext = React.createContext<QuickShare | undefined>(undefined)

export function QuickShareProvider({
  children,
}: {
  children: React.ReactNode
}) {
  const [settings, setSettings] = React.useState<Settings | null>(null)
  const [transfers, setTransfers] = React.useState<Transfer[]>([])
  const [endpoints, setEndpoints] = React.useState<EndpointInfo[]>([])

  React.useEffect(() => {
    api.getSettings().then(setSettings)

    const unlisten = [
      on("rs2js_channelmessage", (cm) =>
        setTransfers((list) => applyMessage(list, cm))
      ),
      on("rs2js_endpointinfo", (ei) =>
        setEndpoints((list) => {
          const others = list.filter((e) => e.id !== ei.id)
          return ei.present ? [...others, ei] : others
        })
      ),
      on("visibility_updated", (visibility) =>
        setSettings((s) => (s ? { ...s, visibility } : s))
      ),
    ]

    return () => unlisten.forEach((p) => p.then((f) => f()))
  }, [])

  const value = React.useMemo<QuickShare>(
    () => ({
      settings,
      updateSettings: (patch) =>
        setSettings((s) => (s ? { ...s, ...patch } : s)),
      transfers,
      endpoints,
      clearEndpoints: () => setEndpoints([]),
      dismiss: (id) => setTransfers((list) => list.filter((t) => t.id !== id)),
      clearFinished: () =>
        setTransfers((list) => list.filter((t) => !isTerminal(t.state))),
    }),
    [settings, transfers, endpoints]
  )

  return (
    <QuickShareContext.Provider value={value}>
      {children}
    </QuickShareContext.Provider>
  )
}

export function useQuickShare() {
  const context = React.useContext(QuickShareContext)
  if (!context) {
    throw new Error("useQuickShare must be used within a QuickShareProvider")
  }
  return context
}
