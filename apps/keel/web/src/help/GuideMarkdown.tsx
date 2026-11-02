/**
 * The kit's `Markdown` with Keel's guides wired in: guide links become `/help` routes and an image
 * under `img/` shows the copy bundled from `docs/guides/keel/img/`. It provides the kit context
 * itself, so it renders the same in a test without the app around it. Load it with `React.lazy`:
 * react-markdown and remark-gfm stay out of the main chunk.
 */
import { KitProvider, Markdown, type KitContext } from '@companion/ui/kit'
import type { ComponentProps } from 'react'

import { KEEL_KIT } from '@/app/kit'

const IMG_DIR = '../../../../../docs/guides/keel/img/'

const IMAGES = import.meta.glob<string>('../../../../../docs/guides/keel/img/*', {
  query: '?url',
  import: 'default',
  eager: true,
})

const GUIDES: KitContext = { ...KEEL_KIT, guideImage: (name) => IMAGES[`${IMG_DIR}${name}`] }

export default function GuideMarkdown(props: ComponentProps<typeof Markdown>) {
  return (
    <KitProvider value={GUIDES}>
      <Markdown {...props} />
    </KitProvider>
  )
}
