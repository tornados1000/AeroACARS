# ADR-0006: Bilingual UI and docs (DE + EN) from day one

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** Project owner

## Context

The project owner is a German speaker; the target user base includes both German-speaking VAs (DACH region) and the wider international phpVMS community.

## Decision

- **UI:** Bilingual **DE + EN** from the first release. i18n infrastructure built in from Phase 1, not retrofitted.
- **Documentation:** Public-facing docs (READMEs, install guides, EULA) in both languages. Internal/architectural docs may be one-language (we'll default to English for code/protocol/architecture, German for product-marketing copy).
- **Code:** **English only.** All identifiers (function names, variables, struct fields, DB columns, error keys) and inline comments must be English. This is non-negotiable for maintainability.
- **Commit messages, PR titles, issue text:** English (one language for project history).

## Implementation guidance

- **Frontend:** [`react-i18next`](https://react.i18next.com/) with two resource bundles per namespace:
  - `client/src/locales/de/<namespace>.json`
  - `client/src/locales/en/<namespace>.json`
- **Default locale:** OS locale on first launch; fallback `en`. Language switcher in Settings.
- **Server module:** Laravel translation files for any user-visible admin strings:
  - `server-module/CloudeAcars/Resources/lang/de/`
  - `server-module/CloudeAcars/Resources/lang/en/`
- **Documentation:** When a doc has both languages, prefer side-by-side sections (`## Title / Titel`) for short docs, separate files (`README.md` + `README.de.md`) for long docs.

## Consequences

- **Positive:** No costly retrofit later when expanding into a second market.
- **Positive:** Forces clean separation of strings from logic from day one.
- **Negative:** Every user-visible string change requires editing two files. Tooling/lint (Phase 4) to catch missing translations is in scope.
