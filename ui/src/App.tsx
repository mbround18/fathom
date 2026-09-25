import { HostedGate } from '@/components/hosted/HostedGate'
import { AppShell } from '@/components/shell/AppShell'
import { TooltipProvider } from '@/components/ui/tooltip'
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
