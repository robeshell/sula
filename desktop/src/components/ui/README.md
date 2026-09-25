# Local shadcn/ui components

Source: official shadcn/ui `new-york-v4` registry, retrieved 2026-09-25.
These source components are owned and maintained inside this application, not loaded at runtime.
See `LICENSE.md` for the original MIT license.

Local changes: `cn` resolves to `src/lib/utils.ts`; registry imports resolve to this folder;
hover accents use the app's muted surface; Button adds `plain` / `none` for media surfaces
and defaults to `type="button"`; Progress forwards its value to the accessible primitive.
`styles/components.css` maps component colors to application-owned appearance preferences.
No external design repository or generated token pipeline is used.

`ModalFrame` composes the Dialog primitives with Motion. Do not use `DialogContent asChild`
with a motion child: its generated close-button siblings are not a single slot child.

Reference: https://ui.shadcn.com/docs/installation/manual
Motion: https://motion.dev/docs/react-accessibility

Typography follows the existing application scale: page titles 24/32, section and dialog
titles 18/24, body and controls 14/20, supporting text 12/16 or 12/18. Input uses
14px at every desktop breakpoint; compact buttons change geometry, not text size.
