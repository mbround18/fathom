/**
 * Markdown, rendered safely: the enrolment instructions (spec 060 FR-012) and the in-app guides
 * (spec 063 R6). Load it with `React.lazy`: react-markdown and remark-gfm stay out of the main
 * chunk.
 *
 * The instructions are written by an admin and shown to whoever enrols a host, so:
 *
 * - raw HTML is never interpreted: a remark plugin turns every `html` node into text, so
 *   `<script>` shows as the characters it is;
 * - links open in a new tab with `rel="noopener noreferrer"`;
 * - images are never fetched: each becomes a link to its URL, labelled with its alt text.
 *
 * `guideLinks` renders a guide from `docs/guides/keel/`, with the same rules plus:
 *
 * - `##` and `###` headings get an `id` from one `slugger()` per render (R7), so `#anchor` works;
 * - a relative link to another guide (`hosts.md#enroll-a-host`) becomes its `/help` route, in the
 *   same tab, and `#anchor` stays on the page;
 * - an image under `img/` shows the copy bundled from `docs/guides/keel/img/`. Any other image is
 *   still a link.
 */
import type { ComponentProps, ReactNode } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { H2, H3, Link } from '@companion/ui/kit'

import { guideHref, isGuideSlug } from '@/help/guides'
import { slugger } from '@/help/slug'

interface MdNode {
  type: string
  value?: string
  depth?: number
  data?: { hProperties?: Record<string, unknown> }
  children?: MdNode[]
}

/** Every raw HTML node becomes a text node with the same characters. */
function remarkHtmlAsText() {
  return (tree: MdNode): void => {
    const walk = (node: MdNode): void => {
      if (node.type === 'html') node.type = 'text'
      node.children?.forEach(walk)
    }
    walk(tree)
  }
}

/** A node's plain text: what its heading reads as. */
function textOf(node: MdNode): string {
  if (node.type === 'text' || node.type === 'inlineCode') return node.value ?? ''
  return (node.children ?? []).map(textOf).join('')
}

/** `##` and `###` get `id = slug(text)`, numbered on repeats, in document order. */
function remarkHeadingIds() {
  return (tree: MdNode): void => {
    const next = slugger()
    const walk = (node: MdNode): void => {
      if (node.type === 'heading' && (node.depth === 2 || node.depth === 3)) {
        node.data = { ...node.data, hProperties: { ...node.data?.hProperties, id: next(textOf(node)) } }
      }
      node.children?.forEach(walk)
    }
    walk(tree)
  }
}

/** Only these schemes are followed; anything else (javascript:, data:) renders as plain text. */
const SAFE = /^(https?:|mailto:|#|\/)/i

const LINK_CLASS = 'text-accent-neon underline underline-offset-2'

/** `hosts.md`, `./hosts.md#enroll-a-host`, `README.md`: a link to another guide. */
const GUIDE_FILE = /^(?:\.\/)?([A-Za-z0-9-]+)\.md(?:#(.*))?$/

/** `img/hosts.png` or `./img/hosts.png`: an image bundled with the guides. */
const GUIDE_IMAGE = /^(?:\.\/)?img\/([^/?#]+)$/

const IMG_DIR = '../../../../../docs/guides/keel/img/'

const IMAGES = import.meta.glob<string>('../../../../../docs/guides/keel/img/*', {
  query: '?url',
  import: 'default',
  eager: true,
})

/** A guide link's in-app route, or `null` when it names no guide. */
function guideRoute(href: string): string | null {
  const m = GUIDE_FILE.exec(href)
  if (!m) return null
  const [, file = '', anchor] = m
  const hash = anchor ? `#${anchor}` : ''
  if (file === 'README') return `/help${hash}`
  return isGuideSlug(file) ? guideHref(anchor ? { guide: file, anchor } : { guide: file }) : null
}

function ExternalLink({ href, children }: { href: string | undefined; children: ReactNode }) {
  return href && SAFE.test(href) ? (
    <a href={href} target="_blank" rel="noopener noreferrer" className={LINK_CLASS}>
      {children}
    </a>
  ) : (
    <span>{children}</span>
  )
}

function ImageLink({ src, alt }: { src: unknown; alt: string | undefined }) {
  const label = alt && alt.length > 0 ? alt : 'image'
  return typeof src === 'string' && SAFE.test(src) ? (
    <a href={src} target="_blank" rel="noopener noreferrer" data-image-link className={LINK_CLASS}>
      [{label}]
    </a>
  ) : (
    <span>[{label}]</span>
  )
}

const components: Components = {
  a: ({ href, children }) => <ExternalLink href={href}>{children}</ExternalLink>,
  img: ({ src, alt }) => <ImageLink src={src} alt={alt} />,
}

const guideComponents: Components = {
  a: ({ href, children }) => {
    if (href?.startsWith('#'))
      return (
        <a href={href} className={LINK_CLASS}>
          {children}
        </a>
      )
    const route = href === undefined ? null : guideRoute(href)
    if (route !== null)
      return (
        <Link to={route} className={LINK_CLASS}>
          {children}
        </Link>
      )
    return <ExternalLink href={href}>{children}</ExternalLink>
  },
  img: ({ src, alt }) => {
    const name = typeof src === 'string' ? GUIDE_IMAGE.exec(src)?.[1] : undefined
    const url = name === undefined ? undefined : IMAGES[`${IMG_DIR}${name}`]
    return url === undefined ? (
      <ImageLink src={src} alt={alt} />
    ) : (
      <img src={url} alt={alt ?? ''} loading="lazy" className="my-2 block h-auto max-w-full border border-border" />
    )
  },
  h2: ({ id, children }) => (
    <h2 id={id} className={`${H2} mt-6 scroll-mt-4`}>
      {children}
    </h2>
  ),
  h3: ({ id, children }) => (
    <h3 id={id} className={`${H3} mt-4 scroll-mt-4`}>
      {children}
    </h3>
  ),
}

const BODY =
  'min-w-0 text-sm/relaxed break-words [&_code]:bg-muted/50 [&_code]:px-0.5 [&_li]:ml-4 [&_ol]:list-decimal [&_p]:my-1.5 [&_pre]:overflow-x-auto [&_pre]:border [&_pre]:border-border [&_pre]:p-2 [&_table]:block [&_table]:overflow-x-auto [&_td]:border [&_td]:border-border [&_td]:px-1 [&_th]:border [&_th]:border-border [&_th]:px-1 [&_ul]:list-disc'

/** The instructions' small headings; a guide's come from the H2/H3 tokens instead. */
const SMALL_HEADINGS = '[&_h1]:text-base [&_h1]:font-semibold [&_h2]:text-sm [&_h2]:font-semibold [&_h3]:font-semibold'

const GUIDE_BODY = '[&_ol]:my-2 [&_table]:my-2 [&_ul]:my-2'

export default function Markdown({
  text,
  guideLinks = false,
  ...rest
}: { text: string; guideLinks?: boolean } & Omit<ComponentProps<'div'>, 'children'>) {
  return (
    <div {...rest} data-markdown className={`${BODY} ${guideLinks ? GUIDE_BODY : SMALL_HEADINGS}`}>
      <ReactMarkdown
        remarkPlugins={guideLinks ? [remarkGfm, remarkHtmlAsText, remarkHeadingIds] : [remarkGfm, remarkHtmlAsText]}
        components={guideLinks ? guideComponents : components}
        skipHtml={false}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}
