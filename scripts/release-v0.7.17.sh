#!/usr/bin/env bash
#
# AeroACARS v0.7.17 Release — Tag + GitHub-Release-Skript
#
# Annahme: PR #3 ist gemerged auf main, du stehst auf einer frischen
# main-Checkout-Kopie auf deinem Pilot-Owner-Rechner.
#
# Was das Skript macht:
#   1. Sanity-Checks: HEAD ist main, working tree clean, Tag existiert noch nicht
#   2. Tag v0.7.17 setzen + pushen
#   3. GitHub-Release anlegen mit Body aus docs/release-notes/v0.7.17.md
#   4. Hinweis auf Server-Deploy + Pilot-QS ausgeben
#
# Was es NICHT macht (manuell, weil sicherheitsrelevant):
#   - Installer bauen (`npm run tauri build`) — separat starten
#   - Server-Deploy (`deploy-recorder.sh` auf live.kant.ovh) — separat starten
#   - Discord-Broadcast an Piloten

set -euo pipefail

VERSION="v0.7.17"
RELEASE_NOTES_FILE="docs/release-notes/v0.7.17.md"

echo "==> AeroACARS ${VERSION} Release-Skript"
echo

# ─── 1. Sanity-Checks ─────────────────────────────────────────────

CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "${CURRENT_BRANCH}" != "main" ]]; then
    echo "❌ FEHLER: aktueller Branch ist '${CURRENT_BRANCH}', erwartet 'main'."
    echo "   Erst PR #3 mergen, dann auf main checkouten."
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

# Sicher-Check: aktueller Commit muss aus dem v0.7.17 PR stammen
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
git tag -a "${VERSION}" -m "${VERSION} Sammel-Release

~15 Bugs aus dem Fenix-Beta-Tester-Feedback adressiert. Highlights:
V/S-led Score-Logik + G-Force-Forensik (B-009), Pilot-Client/Webapp-
Konsistenz (B-015), Cessna-Pattern-FSM (B-010), Fenix AP Master
(B-008), phpVMS-Sperre stoppt Endless-Retry (B-007), aircraft-aware
Bahn-Auslastung (N-002), SimBrief-Refresh wirkt (N-001), Auto-Start-
Diagnose (N-003), Fenix Auto-Detect (F-001) + Squawk-Suppress (B-002)
+ Aircraft-Type-Anzeige (B-001) + B-003 / B-004 / B-005 / B-006.

Notes: ${RELEASE_NOTES_FILE}
Tracker: docs/qs/v0.7.16-fenix-beta-bugs.md"

echo "==> Tag nach origin pushen..."
git push origin "${VERSION}"

# ─── 3. GitHub-Release anlegen ────────────────────────────────────

echo
echo "==> GitHub-Release ${VERSION} anlegen..."
gh release create "${VERSION}" \
    --title "${VERSION} — Sammel-Release: Score-Konsistenz + Fenix + Stability" \
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
echo "        gh release upload ${VERSION} client/src-tauri/target/release/bundle/nsis/AeroACARS_0.7.17_x64-setup.exe"
echo
echo "   2. aeroacars-live deployen (Webapp + Recorder):"
echo "        cd /pfad/zu/aeroacars-live"
echo "        ./deploy-recorder.sh"
echo "      → systemctl status aeroacars-recorder muss 'active (running)' zeigen"
echo
echo "   3. Pilot-QS gegen die finalen Befunde des QS-Round-3 fahren."
echo
echo "   4. Discord-Broadcast an Piloten mit:"
echo "        - Link zum Release: https://github.com/MANFahrer-GF/AeroACARS/releases/tag/${VERSION}"
echo "        - Hinweis: Update via Auto-Updater im Client, oder manueller Installer-Download"
echo
echo "==> Fertig."
