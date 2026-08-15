# UI/UX Skills used by FastExplorer

FastExplorer keeps a project-local set of UI/UX Agent Skills in `.agents/skills/`. They are references for design and review, not framework dependencies.

## Installed references

### frontend-design — Anthropic

Use for visual direction and intentionality. The useful principle for FastExplorer is that every structural or visual choice should serve the product's job instead of looking like a generic generated interface.

Native adaptation: FastExplorer should stay recognizably a desktop file explorer. Distinctiveness belongs in theme quality, spacing, hierarchy, and detail rather than inventing unfamiliar navigation patterns.

### web-design-guidelines — Vercel

Use for systematic interaction and UX review. Apply framework-agnostic rules such as keyboard operability, visible focus, forgiving hit targets, optical alignment, no dead zones, resilient content, and designed empty/error states.

Native adaptation: translate HTML/ARIA examples into Xilem and AccessKit equivalents. Browser-only guidance is not an implementation requirement.

### make-interfaces-feel-better — Jakub Krehel

Use for final detail work: concentric radii, optical alignment, structural borders, hit areas, icon consistency, restrained motion, and checking every interaction state.

For FastExplorer, platform density and Explorer conventions override generic mobile/web sizing when they conflict, but small visible controls still need forgiving non-overlapping hit targets.

### fixing-accessibility — ibelick/ui-skills

Use whenever interactive controls, tabs, focus, keyboard behavior, inputs, dialogs, or state cues change. Prioritize meaningful accessible names, keyboard reachability, visible focus, and redundant state cues.

Native adaptation: use Xilem's native buttons/focus behavior and AccessKit semantics. Do not add web ARIA concepts where the native widget already supplies the semantic role.

## Required UI workflow

For non-trivial UI changes:

1. Inspect existing FastExplorer tokens and the relevant desktop convention.
2. Read the relevant installed Skill(s) above.
3. Make the smallest coherent change; do not import a second styling or widget system for polish.
4. Run `cargo fmt --check`, `cargo check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
5. Run the real app and inspect the changed state visually.
6. Put user-review captures only in `screenshots/review/`; failed or diagnostic captures go in `screenshots/internal/`.
7. Check hover/focus/active/disabled states where the Xilem widgets expose them.

When Skill advice conflicts with a native desktop convention, document the reason and prefer the established desktop interaction unless the user explicitly requests a redesign.

## Additional review references

The following project-local Skills are also installed for deeper UI work. They are references only; native Xilem and desktop/Android conventions remain authoritative.

- `frontend-design-review` — Microsoft: structured responsive, accessibility, consistency, and visual-quality review.
- `frontend-ui-engineering` — Addy Osmani: production UI engineering, responsive states, loading/error handling, and resilient interaction design.
- `mobile-ui-ux-designer` — mobile information hierarchy, adaptive density, touch behavior, disabled states, and mobile layout review.
- `frontend-design-principles` — application/product UI principles, especially forms, settings, data-heavy views, and progressive disclosure.
- `mobile-accessibility` — mobile-specific target, state, focus, and assistive-technology checks.
- `baseline-ui` — a compact baseline against generic card-heavy, low-density, or inconsistent generated UI.
- `ux-layout` — Microsoft VS Code layout guidance for dense application surfaces, scrolling, truncation, and constrained-space behavior.

For FastExplorer, use these to challenge a design before implementation, not to copy web-specific components. In particular: prefer aligned full-width setting controls over intrinsic text-width pills; use compact anchored menus rather than modal dialogs for command overflow; use dense one-row status lists for high-volume transfer activity; and preserve fixed placement for structural controls such as Settings and More.
