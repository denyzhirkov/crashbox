// Global open-state for the command palette so the top bar (⌘K button, project
// chip) and the keyboard shortcut all drive the same surface.
import { createSignal } from 'solid-js'

export const [paletteOpen, setPaletteOpen] = createSignal(false)
