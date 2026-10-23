import { HostedGate } from '@/components/hosted/HostedGate'
import { AppShell } from '@/components/shell/AppShell'
import { TooltipProvider } from '@companion/ui'
import { useBackend } from '@/protocol/use-backend'

export default function App() {
  useBackend()
  return (
    <TooltipProvider delayDuration={400}>
      <HostedGate>
        <AppShell />
      </HostedGate>
    </TooltipProvider>
  )
}
