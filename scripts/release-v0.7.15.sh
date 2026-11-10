#!/usr/bin/env bash
#
# AeroACARS v0.7.15 Sim-Recovery Release — Tag + GitHub-Release-Skript
#
# Annahme: PR #1 ist gemerged auf main, du stehst auf einer frischen
# main-Checkout-Kopie auf deinem Pilot-Owner-Rechner.
#
# Was das Skript macht:
#   1. Sanity-Checks: HEAD ist main, working tree clean, Tag existiert noch nicht
#   2. Tag v0.7.15 setzen + pushen
#   3. GitHub-Release anlegen mit Body aus docs/release-notes/v0.7.15.md
#   4. Hinweis auf Server-Deploy + Pilot-QS ausgeben
#
# Was es NICHT macht (manuell, weil sicherheitsrelevant):
#   - Installer bauen (`npm run tauri build`) — separat starten
#   - Server-Deploy (`deploy-recorder.sh` auf live.kant.ovh) — separat starten
#   - Discord-Broadcast an Piloten

set -euo pipefail

VERSION="v0.7.15"
RELEASE_NOTES_FILE="docs/release-notes/v0.7.15.md"
QS_CHECKLIST_FILE="docs/qs/v0.7.15-qs-checklist.md"

echo "==> AeroACARS ${VERSION} Release-Skript"
echo

# ─── 1. Sanity-Checks ─────────────────────────────────────────────

CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "${CURRENT_BRANCH}" != "main" ]]; then
    echo "❌ FEHLER: aktueller Branch ist '${CURRENT_BRANCH}', erwartet 'main'."
    echo "   Erst PR #1 mergen, dann auf main checkouten."
    exit 1
fi

if [[ -n $(git status --porcelain) ]]; then
    echo "❌ FEHLER: working tree nicht clean. Bitte erst Änderungen committen oder stashen."
    git status --short
    exit 1
fi

if git rev-parse "${VERSION}" >/dev/null 2>&1; then
    echo "❌ FEHLER: Tag ${VERSION} existiert bereits."
    echo "   Existing tag points at: $(git rev-list -n 1 ${VERSION})"
    echo "   Falls du es löschen willst (Vorsicht, destruktiv):"
    echo "     git tag -d ${VERSION}"
    echo "     git push origin :refs/tags/${VERSION}"
    exit 1
fi

if [[ ! -f "${RELEASE_NOTES_FILE}" ]]; then
    echo "❌ FEHLER: ${RELEASE_NOTES_FILE} nicht gefunden."
    echo "   Bist du im Repo-Root von AeroACARS?"
    exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
    echo "❌ FEHLER: 'gh' (GitHub CLI) nicht installiert."
    echo "   Installation: https://cli.github.com/"
    exit 1
fi

# Sicher-Check: aktueller Commit muss im PR #1 enthalten sein
HEAD_SHA=$(git rev-parse HEAD)
echo "==> HEAD: ${HEAD_SHA} (main)"

# Bestätigung
echo
echo "==> Was passiert jetzt:"
echo "    1. Tag ${VERSION} setzen auf ${HEAD_SHA:0:7}"
echo "    2. Tag nach origin pushen"
echo "    3. GitHub-Release ${VERSION} anlegen mit Notes aus ${RELEASE_NOTES_FILE}"
echo "    4. Hinweis auf nachfolgende manuelle Schritte ausgeben"
echo
read -rp "Fortfahren? [y/N] " ANSWER
if [[ "${ANSWER}" != "y" && "${ANSWER}" != "Y" ]]; then
    echo "abgebrochen."
    exit 0
fi

# ─── 2. Tag setzen + pushen ───────────────────────────────────────

echo
echo "==> Tag ${VERSION} setzen..."
git tag -a "${VERSION}" -m "${VERSION} Sim-Recovery Release

Auto-Resume + Pause-Akkumulator + pirep_id-Server-Join +
F5 MSFS Pause_EX1 + F6 X-Plane Pause/Replay + F7 Aircraft-Change.

Trigger: PIREP J2VoaZmoD6LQGpMg (AUA 323 LOWW->ESGG am 2026-05-11).
Spec: docs/spec/sim-disconnect-auto-resume.md
Notes: ${RELEASE_NOTES_FILE}
QS:    ${QS_CHECKLIST_FILE}"

echo "==> Tag nach origin pushen..."
git push origin "${VERSION}"

# ─── 3. GitHub-Release anlegen ────────────────────────────────────

echo
echo "==> GitHub-Release ${VERSION} anlegen..."
gh release create "${VERSION}" \
    --title "${VERSION} — Sim-Recovery Release" \
    --notes-file "${RELEASE_NOTES_FILE}" \
    --verify-tag

# ─── 4. Nächste Schritte ──────────────────────────────────────────

echo
echo "✓ Tag + GitHub-Release ${VERSION} angelegt."
echo
echo "==> Jetzt manuell:"
echo
echo "   1. Installer bauen:"
echo "        cd client"
echo "        npm install"
echo "        npm run tauri build"
echo "      → NSIS-Installer landet in client/src-tauri/target/release/bundle/nsis/"
echo "      → Installer-EXE per gh release upload anhängen:"
echo "        gh release upload ${VERSION} client/src-tauri/target/release/bundle/nsis/AeroACARS_0.7.15_x64-setup.exe"
echo
echo "   2. Server-Recorder auf live.kant.ovh deployen:"
echo "        cd /pfad/zu/aeroacars-live"
echo "        ./deploy-recorder.sh"
echo "      → systemctl status aeroacars-recorder muss 'active (running)' zeigen"
echo
echo "   3. Pilot-QS gegen die Checkliste fahren:"
echo "        cat ${QS_CHECKLIST_FILE}"
echo "      → Block G1 (Re-Trigger AUA-323-Szenario) ist der finale Akzeptanz-Test."
echo
echo "   4. Discord-Broadcast an Piloten mit:"
echo "        - Link zum Release: https://github.com/MANFahrer-GF/AeroACARS/releases/tag/${VERSION}"
echo "        - Hinweis: Update via Auto-Updater im Client, oder manueller Installer-Download"
echo
echo "==> Fertig."
