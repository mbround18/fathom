/**
 * What Keel hands the shared kit (spec 065): how to reach its guides, which Markdown links name
 * one, and the glossary its `TermTip`s read.
 */
import type { KitContext } from '@companion/ui/kit'

import { GLOSSARY } from '@/help/glossary'
import { guideHref, isGuide } from '@/help/guides'

export const KEEL_KIT: KitContext = { guideHref, isGuide, glossary: GLOSSARY }
