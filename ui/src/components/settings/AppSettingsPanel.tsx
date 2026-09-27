/**
 * 043 US3: Settings › App settings. The app-wide options that used to sit inside Cluster
 * settings: appearance, the AI assistant and the agents that can reach Fathom (015). None of
 * them depend on a context, so the page opens with no cluster connected.
 */
import { AppearanceSettings } from '@/components/panels/AppearanceSettings'
import { AvailableAgentsSection } from '@/components/panels/agent-ops/AvailableAgentsSection'
import { AiAssistSettings } from '@/features/ai-assist/AiAssistSettings'
import { useStore } from '@/store'

export function AppSettingsPanel() {
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  return (
    <div
      data-testid="app-settings-panel"
      className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4"
    >
      <AppearanceSettings />
      {!hosted && (
        <>
          <AiAssistSettings />
          <AvailableAgentsSection />
        </>
      )}
    </div>
  )
}
