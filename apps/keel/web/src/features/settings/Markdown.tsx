/**
 * The enrolment instructions, rendered (spec 060 FR-012). Load it with `React.lazy`: react-markdown
 * and remark-gfm stay out of the main chunk.
 *
 * The text is written by an admin and shown to whoever enrols a host, so it renders safely:
 *
 * - raw HTML is never interpreted: a remark plugin turns every `html` node into text, so
 *   `<script>` shows as the characters it is;
 * - links open in a new tab with `rel="noopener noreferrer"`;
 * - images are never fetched: each becomes a link to its URL, labelled with its alt text.
 */
import type { ComponentProps } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'

interface MdNode {
  type: string
  value?: string
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

/** Only these schemes are followed; anything else (javascript:, data:) renders as plain text. */
const SAFE = /^(https?:|mailto:|#|\/)/i

const components: Components = {
  a: ({ href, children }) =>
    href && SAFE.test(href) ? (
      <a href={href} target="_blank" rel="noopener noreferrer" className="text-accent-neon underline underline-offset-2">
        {children}
      </a>
    ) : (
      <span>{children}</span>
    ),
  img: ({ src, alt }) => {
    const label = alt && alt.length > 0 ? alt : 'image'
    return typeof src === 'string' && SAFE.test(src) ? (
      <a
        href={src}
        target="_blank"
        rel="noopener noreferrer"
        data-image-link
        className="text-accent-neon underline underline-offset-2"
      >
        [{label}]
      </a>
    ) : (
      <span>[{label}]</span>
    )
  },
}

export default function Markdown({ text, ...rest }: { text: string } & Omit<ComponentProps<'div'>, 'children'>) {
  return (
    <div
      {...rest}
      data-markdown
      className="min-w-0 text-xs/relaxed break-words [&_code]:bg-muted/50 [&_code]:px-0.5 [&_h1]:text-sm [&_h1]:font-semibold [&_h2]:text-xs [&_h2]:font-semibold [&_h2]:uppercase [&_h3]:font-semibold [&_li]:ml-4 [&_ol]:list-decimal [&_p]:my-1.5 [&_pre]:overflow-x-auto [&_pre]:border [&_pre]:border-border [&_pre]:p-2 [&_table]:block [&_table]:overflow-x-auto [&_td]:border [&_td]:border-border [&_td]:px-1 [&_th]:border [&_th]:border-border [&_th]:px-1 [&_ul]:list-disc"
    >
      <ReactMarkdown remarkPlugins={[remarkGfm, remarkHtmlAsText]} components={components} skipHtml={false}>
        {text}
      </ReactMarkdown>
    </div>
  )
}
