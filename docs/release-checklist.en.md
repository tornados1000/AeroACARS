# Pre-release checklist — AeroACARS

**MUST** be walked through before every `git tag vX.Y.Z && git push origin vX.Y.Z`. Check off each line or explicitly override it (document why in the release PR body).

Background: v0.9.0/v0.9.1/v0.9.2 had a **release-blocking update-modal bug** that only surfaced at the end user (Discord report by Svenny1974, 2026-05-18) — the modal blew past the viewport, raw Markdown wasn't rendered. Had we had this checklist + the vitest guards that now exist, the bug would have gone red BEFORE release.

---

## 0. Branch hygiene

- [ ] Branch is rebased onto current `main` (not an 8-month-old branch like the v0.10.0 feature branch that produced the StableApproachBanner diff)
- [ ] `git diff origin/main..HEAD --stat` contains ONLY files that really belong to this release
- [ ] No `dev-only` artifacts in `git diff --cached` (e.g. ad-hoc test buttons, Sentry test endpoints)

## 1. Version bump in sync at all three places

- [ ] `client/package.json` `version`
- [ ] `client/src-tauri/Cargo.toml` `[workspace.package] version`
- [ ] `client/src-tauri/tauri.conf.json` `version`
- [ ] `Cargo.lock` automatically updated along with it (cargo run / check)

Check: `grep -E '"0\.X\.Y"|version = "0\.X\.Y"' client/package.json client/src-tauri/Cargo.toml client/src-tauri/tauri.conf.json` shows **all three** with the new version.

## 2. Bilingual release notes

- [ ] `docs/release-notes/vX.Y.Z.md` exists
- [ ] Has a 🇩🇪 German block AND a 🇬🇧 English block
- [ ] If wire-format changes are included: VA-owner note on whether `aeroacars-live` must be deployed alongside
- [ ] If a migration-sensitive change is included (score algorithms, DB schemas): a concrete note on what should happen

## 3. Tests green

- [ ] `cargo check` (`client/src-tauri/`) without warnings/errors
- [ ] `cargo test -p landing-scoring`
- [ ] `cargo test -p aeroacars-app --lib` (backend lib tests)
- [ ] `cargo test --doc` (**doctests!** — v0.19.3 left `main` red because an
      indented example text in the module comment of `arrival.rs` was
      interpreted as Rust code and an attempt was made to compile it.
      `--lib` does NOT catch this.)
- [ ] `npm test` in `client/` (Vitest)
- [ ] `npx tsc -b` in `client/` (strict type check)

**Specifically mandatory for every release:**

- [ ] `UpdateButton.test.tsx` is green — prevents a repeat of the Svenny bug (modal structure + Markdown parsing). If red: do NOT release, fix first.

## 4. Update-modal smoke test (manual, 60 seconds)

Sounds trivial, but would have caught the Svenny bug. Before EVERY release:

- [ ] Start `npm run dev`
- [ ] In the DevTools console: inject a mock update to force the modal — e.g. modify the component state of `UpdateButton` via React DevTools, or patch the `useUpdateChecker` hook locally with a stub
- [ ] **Open with the CURRENT release body** (= content from `docs/release-notes/vX.Y.Z.md`)
- [ ] Visual check:
  - [ ] Modal stays within the viewport (not past the screen edge)
  - [ ] "Install" button visible at the bottom edge
  - [ ] Notes scroll when long
  - [ ] Markdown is rendered (no `###`, no `**bold**` raw strings, no table pipes as text)

Alternative if mock injection is too much effort: manually install an old version on a second PC, push the new tag, wait until the updater offers it, open the modal and click through.

## 5. Wire compatibility (if score/payload/DB changes are included)

- [ ] `aeroacars-live` branch exists with the mirror implementation
- [ ] aeroacars-live `tsc --noEmit` green
- [ ] aeroacars-live `landingScoring.test.ts` green
- [ ] **Order:** deploy aeroacars-live FIRST (VPS `deploy-recorder.sh`), THEN push the pilot-client tag. Otherwise freshly updated pilots see fields the recorder doesn't yet forward.

## 6. v0.9.x update path (one-time hotfix)

As long as v0.9.x clients are still out there:

- [ ] Discord announcement clearly states that v0.9.x users must install the new Setup.exe from GitHub Releases **manually** (the modal in v0.9.x is broken and can't trigger the auto-update)
- [ ] v0.9.2 (and possibly v0.9.1, v0.9.0) marked as `--prerelease` via `gh release edit v0.9.2 --prerelease`, so that the updater on existing installations sees v0.10.0 as "latest"

From v0.10.0+ the auto-update works normally again (modal hotfix in v0.10.0).

## 7. Release tag + GitHub Actions

- [ ] PR merged onto `main`
- [ ] `git tag vX.Y.Z && git push origin vX.Y.Z` (NO local `npm run tauri build` — GitHub Actions builds signed Win+Mac, see `MEMORY.md` "Release automation")
- [ ] Copy the GitHub release body in from `docs/release-notes/vX.Y.Z.md` (that exact file then also renders cleanly in the update modal — see step 4)
- [ ] Verify in the release assets:
  - [ ] `latest.json` contains the new version
  - [ ] `AeroACARS_x64-setup.exe` + signatures present

## 8. Post-release verification

- [ ] On a test machine with the **previous** version: have the update modal offered, click through, check that it installs
- [ ] For wire-format changes: fly 1 test touchdown live and verify it in the aeroacars-live dashboard

---

## Why this checklist exists

Discord, 2026-05-18, pilot Svenny1974:
> Wanted to do the update, but nothing happens beyond what you see in the picture. Can't scroll, nothing.

**Root cause:** `.update-modal` in v0.9.x had no `max-height` + no `overflow`, plus `update.body` was rendered as a raw `<p>`. With a long bilingual body this blew the modal past the viewport — the install button was below the fold.

**What would have prevented the bug (= now structurally in the repo):**
1. This checklist (step 4 would have flagged it)
2. `client/src/components/UpdateButton.test.tsx` (step 3 would have gone red)
3. WARN comments in `App.css` `.update-modal*` (step 0 would have prevented someone from accidentally removing max-height)

Walking through this checklist takes 5 minutes. A released bug that stops pilots from updating costs hours of Discord support + another release. We walk through it.
