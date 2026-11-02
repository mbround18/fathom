/**
 * What Keel hands the shared kit (spec 065): how to reach its guides and which Markdown links name
 * one.
 */
import type { KitContext } from '@companion/ui/kit'

import { guideHref, isGuide } from '@/help/guides'

export const KEEL_KIT: KitContext = { guideHref, isGuide }
