# Releasing AeroACARS

This is the manual release workflow until we set up GitHub Actions automation.

## Pre-flight checks

- [ ] All work merged into `main`
- [ ] Working tree clean (`git status`)
- [ ] Tested locally (`npm run tauri dev`) — at minimum, login + sim adapter starts
- [ ] Decided on the version bump (semver: MAJOR.MINOR.PATCH)

## Step 1 — Bump version

Edit two files:
- `client/src-tauri/Cargo.toml` → `[workspace.package].version`
- `client/src-tauri/tauri.conf.json` → `"version"`

Both must match. Commit the bump:

```bash
git commit -am "release: vX.Y.Z"
git tag -a vX.Y.Z -m "AeroACARS vX.Y.Z"
git push --follow-tags
```

## Step 2 — Build the signed installer

The private signing key is at `client/aeroacars-updater.key` (NEVER committed).
Tauri 2 wants the key **content** (not a path) in the env var:

```powershell
cd client
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content aeroacars-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""    # we generated keys without a password
npm run tauri build -- --bundles nsis
```

(The `_PATH` variant in Tauri docs is for older versions; v2's
plugin-updater build hook reads `TAURI_SIGNING_PRIVATE_KEY` as the
literal key text.)

This produces three files in `client/src-tauri/target/release/bundle/nsis/`:

- `AeroACARS_X.Y.Z_x64-setup.exe`         ← the installer
- `AeroACARS_X.Y.Z_x64-setup.exe.sig`     ← Ed25519 signature
- `latest.json`                           ← updater manifest

## Step 3 — Create the GitHub release

1. Go to <https://github.com/MANFahrer-GF/AeroACARS/releases/new>
2. Choose tag: `vX.Y.Z`
3. Title: `AeroACARS vX.Y.Z`
4. Description: changelog (use commit messages since the previous tag)
5. **Upload all three files** from Step 2 as release assets
6. Check "Set as the latest release"
7. Publish

The Tauri updater queries
`https://github.com/MANFahrer-GF/AeroACARS/releases/latest/download/latest.json`
which GitHub redirects to the latest release's `latest.json` asset.

## Step 4 — Smoke test

On a fresh machine (or after deinstalling the previous version):

1. Download `AeroACARS_X.Y.Z_x64-setup.exe` from the release
2. Run the installer
3. Start AeroACARS, log in, verify version in **Über** tab
4. To verify auto-update: bump the version locally (Step 1), build (Step 2),
   create another release (Step 3), then start the OLD installed version —
   the update banner should appear within ~3 seconds of app startup.

## Verifying the signature manually

```bash
npx @tauri-apps/cli signer verify -k aeroacars-updater.key.pub -s AeroACARS_X.Y.Z_x64-setup.exe.sig AeroACARS_X.Y.Z_x64-setup.exe
```

Should print `Signature OK`.

## What to do if you lose the private key

Game over — every machine running AeroACARS will refuse the next update because
the new signature won't match the embedded public key. To recover:

1. Generate a new keypair (`npx @tauri-apps/cli signer generate`)
2. Update `tauri.conf.json` → `plugins.updater.pubkey`
3. Bump major version (compatibility break)
4. Pilots have to manually download + reinstall the new version once.
   After that, auto-update works again.

So keep the private key safe — backup, password manager, etc.
