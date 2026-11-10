# Releasing AeroACARS

Releases are **automated** via GitHub Actions. You push a tag, the
CI builds Windows + macOS in parallel, signs both with the Tauri
updater key, and publishes a GitHub release with all artifacts.

## One-time setup (already done — recorded for the future)

### 1. Add the signing key as a GitHub Secret

Repo Settings → Secrets and variables → Actions → New repository secret:

| Name | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Full contents of `client/aeroacars-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | (empty — we generated keys without a password) |

The CI passes these as env vars to `tauri-action`, which signs both
the `.exe` (Windows) and the `.app.tar.gz` (macOS) with the same key.

The PUBLIC key in `client/src-tauri/tauri.conf.json` →
`plugins.updater.pubkey` is what installed clients verify against —
it stays in the repo (it's public by design).

### 2. Verify the workflow file

`.github/workflows/release.yml` is the source of truth. It:
- Triggers on tag push (`v*`) or manual `workflow_dispatch`
- Runs two jobs in parallel:
  - `build` on `windows-latest` → NSIS installer
  - `build` on `macos-latest` → .dmg + .app.tar.gz (Apple Silicon)
- Uses `tauri-apps/tauri-action@v0` to do the actual build + sign + upload
- A final `publish` job promotes the draft release to public

## Cutting a release

### Step 1 — Bump version

Two files MUST stay in sync:

```
client/src-tauri/Cargo.toml      → [workspace.package].version
client/src-tauri/tauri.conf.json → "version"
```

### Step 2 — Commit + tag

```bash
git add -A
git commit -m "release: vX.Y.Z — <one-line summary>"
git tag -a vX.Y.Z -m "AeroACARS vX.Y.Z"
git push --follow-tags
```

That's it. GitHub Actions takes over.

### Step 3 — Watch the CI

<https://github.com/MANFahrer-GF/AeroACARS/actions>

Two parallel build jobs (~5-8 min each) followed by the publish
job. Total wall time: ~10 min from `git push --tags` to release
visible to users.

### Step 4 — Edit the release notes

The auto-generated release body is generic. Open
<https://github.com/MANFahrer-GF/AeroACARS/releases/tag/vX.Y.Z>
and edit the body to describe what changed.

The Tauri updater doesn't read the GitHub release body — it reads
`latest.json`'s `notes` field. To customize that, edit
`scripts/update-notes.txt` (TODO — for now it's hardcoded in the
workflow).

## Manual local builds (for debugging / pre-CI testing)

If you need to reproduce a build locally without going through CI:

```powershell
cd client
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content aeroacars-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
npm run tauri build -- --bundles nsis
```

This produces:
- `client/src-tauri/target/release/bundle/nsis/AeroACARS_X.Y.Z_x64-setup.exe`
- `client/src-tauri/target/release/bundle/nsis/AeroACARS_X.Y.Z_x64-setup.exe.sig`

For macOS testing you'd need a Mac. The CI handles that for you.

## Verifying signatures

```powershell
npx @tauri-apps/cli signer verify -k aeroacars-updater.key.pub -s AeroACARS_X.Y.Z_x64-setup.exe.sig AeroACARS_X.Y.Z_x64-setup.exe
```

Should print `Signature OK`.

## Recovery — if you lose the private key

Game over for the Updater-Trust-Chain — every machine running
AeroACARS will refuse the next update because the new signature
won't match the embedded public key. To recover:

1. Generate a new keypair (`npx @tauri-apps/cli signer generate`)
2. Update `tauri.conf.json` → `plugins.updater.pubkey`
3. Update GitHub Secret `TAURI_SIGNING_PRIVATE_KEY` with the new private key
4. Bump major version (compatibility break)
5. Pilots have to manually download + reinstall the new version once.
   After that, auto-update works again.

So keep the private key safe — backup, password manager, etc.
