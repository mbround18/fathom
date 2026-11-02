/**
 * Settings → System → Build (spec 061 US1, FR-001, FR-002): what this Keel is running, for owners
 * and admins. Version, revision (short, the full one on hover and Copy), image digest (likewise) and
 * build date. A build without a version or revision says "development build"; a deployment that
 * didn't pass a digest says "unknown" with a one-line hint to pin one.
 */
import { CubeIcon } from '@phosphor-icons/react'
import { useEffect } from 'react'
import { H2, LoadState } from '@companion/ui/kit'

import type { KeelBuild } from '@/protocol/wire'

import { Row, ShortCopy, Time, Unknown } from './build-parts'
import { shortDigest, shortRevision, useUpdates } from './updates'

const DEV = 'development build'

function Values({ build }: { build: KeelBuild }) {
  const dev = build.version === null && build.revision === null
  return (
    <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-0.5 text-xs">
      <Row term="Version" data="version">
        <span className="font-mono">{build.version ?? build.display}</span>
      </Row>
      <Row term="Revision" data="revision">
        {build.revision ? (
          <ShortCopy label="revision" short={shortRevision(build.revision)} value={build.revision} />
        ) : (
          <Unknown text={dev ? DEV : 'unknown'} />
        )}
      </Row>
      <Row term="Digest" data="digest">
        {build.digest ? (
          <ShortCopy label="digest" short={shortDigest(build.digest)} value={build.digest} />
        ) : (
          <span className="flex min-w-0 flex-col py-1">
            <Unknown />
            <span data-hint="pin-digest" className="text-xs text-status-warn">
              Deploy Keel pinned to an image digest and pass it as KEEL_IMAGE_DIGEST, so a rollback can name it.
            </span>
          </span>
        )}
      </Row>
      <Row term="Built" data="built">
        {build.built_at ? <Time at={build.built_at} /> : <Unknown text={dev ? DEV : 'unknown'} />}
      </Row>
    </dl>
  )
}

export function BuildPanel() {
  const build = useUpdates((s) => s.build)
  const refusal = useUpdates((s) => s.buildRefusal)
  const load = useUpdates((s) => s.loadBuild)
  useEffect(() => {
    load()
  }, [load])
  return (
    <section aria-label="Running build" data-build-panel className="flex min-w-0 flex-col gap-2 border border-border p-3">
      <h2 className={`${H2} flex items-center gap-1.5`}>
        <CubeIcon aria-hidden className="size-3.5 text-accent-neon" />
        Running build
      </h2>
      <LoadState loading={build === null} failed={refusal} what="the running build" onRetry={load} label="Reading the running build…">
        {build && <Values build={build} />}
      </LoadState>
    </section>
  )
}
