# FastExplorer Agent Guidance

## UI / UX work

Before changing user-facing UI, read `docs/ui-ux-skills.md` and the relevant local skills under `.agents/skills/`.

Use these by default:

- `.agents/skills/frontend-design/SKILL.md` for visual direction and intentional design choices.
- `.agents/skills/web-design-guidelines/SKILL.md` for interaction, layout, content, and usability review.
- `.agents/skills/make-interfaces-feel-better/SKILL.md` for surfaces, hit targets, optical alignment, icons, and polish.
- `.agents/skills/fixing-accessibility/SKILL.md` whenever controls, tabs, keyboard behavior, focus, inputs, or state cues change.

FastExplorer is a native Xilem desktop application. Translate web-specific guidance into native Xilem / AccessKit equivalents; do not add HTML, ARIA, React, CSS, or browser-only machinery to satisfy a web-specific example.

Priority order when guidance conflicts:

1. Explicit user requirement.
2. Established desktop / Explorer interaction convention.
3. Existing FastExplorer design tokens and behavior.
4. Native Xilem / AccessKit capability.
5. General principles from the installed skills.
6. Web-specific implementation details only when conceptually applicable.

For UI changes, compile and run the real app, inspect a screenshot, and test relevant interaction states. Put screenshots intended for the user in `screenshots/review/`; put diagnostic or failed captures in `screenshots/internal/`.

Do not create decorative UI that competes with file-management tasks. Prefer clear hierarchy, predictable targets, visible state, and restrained motion. Preserve performance and dense desktop usability.

Xilem 0.4 Flex has a 10px default gap. Always set `.gap(...)` explicitly on every `flex_row` / `flex_col`; use `0.px()` for structural containers and explicit `FlexSpacer` or a deliberate non-zero gap where separation is intended. Never rely on the framework default for application spacing.

Use progressive disclosure as a standing rule: frequent file-management actions belong on the primary surface with the shortest practical path; infrequent view options, backend choices, diagnostics, and configuration belong behind `More` / Settings. Do not promote low-frequency controls into the main chrome merely because space is available.
