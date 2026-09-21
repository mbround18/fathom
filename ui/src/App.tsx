import { AppShell } from '@/components/shell/AppShell'
import { TooltipProvider } from '@/components/ui/tooltip'

export default function App() {
  return (
    <TooltipProvider delayDuration={400}>
      <AppShell />
    </TooltipProvider>
  )
}
