# Releasing Hex

One run of the **Release** workflow (`.github/workflows/release-desktop.yml`)
builds, signs, and publishes every platform from one commit.

```ts
Release (workflow_dispatch: ref, channel; or push tag app-v<version>)
├─ resolve        version from Cargo.toml, channel, key prefix, tag
├─ build          build-desktop.yml: Linux x86_64 (Arch container), Windows x64 + ARM64,
│                 Authenticode via Azure Artifact Signing (docs/windows-signing.md)
├─ macos          build-macos.yml: Developer ID, notarized DMG, Sparkle ZIP, signed appcast
│                 (docs/plans/macos-signing.md)
├─ sign           ed25519-signs the Linux/Windows feeds (never compiles); stable
│                 requires Authenticode signatures
├─ publish        R2 + GitHub release (scripts/publish-release.sh)
└─ aur            builds, installs, and tests hex-voice-bin; stable pushes it
```

## Channels

- `staging` (the default) is the dry run. Every object lands under
  `staging/<run-id>/` in the bucket (`https://downloads.hex.anoma.ly/staging/<run-id>/…`),
  the GitHub release is a draft prerelease tagged `staging-<run-id>`, and the
  AUR package is built and installed in a container but not pushed. Installed
  apps never read the prefix. To try a staged update on Linux or Windows, run
  Hex with `HEX_RELEASE_BASE_URL=https://downloads.hex.anoma.ly/staging/<run-id>`
  (HTTPS only; the compiled key still decides what is trusted). A staging run
  without the macOS build publishes Linux and Windows only.
- `stable` publishes the public release: root keys, a published GitHub release
  `app-v<version>` (kept off "Latest", which the SDK uses), and the AUR push.
  It needs every platform, Authenticode-signed Windows executables, and
  `docs/releases/<version>.md`. Pushing tag `app-v<version>` runs it too.

## Cutting a release

1. Bump `version` in `Cargo.toml` (and `Cargo.lock`), add
   `docs/releases/<version>.md`, push to `main`.
2. Run Release with `channel: staging`; check the summary, the draft release,
   and, if useful, a staged update.
3. Run Release with `channel: stable` from `main`.
4. Bump `MAC_VERSION`/`DESKTOP_VERSION` in `site/src/release.ts`, the README
   version link, and the Homebrew cask (`anomalyco/homebrew-tap`,
   `Casks/hex.rb`: version and the DMG's sha256).

The macOS build number is `major*10000 + minor*100 + patch`; Sparkle compares
it, so it must exceed every published and distributed beta build. The feeds
refuse an equal or older version.

## Hosting

Everything is served from the `hex-releases` R2 bucket on Anomaly's Cloudflare
account through the custom domain `https://downloads.hex.anoma.ly`:

```ts
downloads.hex.anoma.ly
├─ appcast.xml                      Sparkle feed (no-cache)
├─ linux-update.json                signed feeds (no-cache)
├─ windows-update.json / windows-aarch64-update.json
├─ install-linux.sh / install-windows.ps1
└─ releases/
   ├─ HEX-<v>-<sha256>-<target>     content-addressed self-update artifacts
   ├─ HEX-<v>-arm64.dmg / .zip      macOS download / Sparkle archive
   ├─ HEX-Setup-<v>-<x64|arm64>.exe Windows installers
   └─ HEX-latest-arm64.dmg          moving pointer (no-cache)
```

CI uploads through the bucket's S3 API with the account token
`github-actions-hex` (Workers R2 Storage Bucket Item Read/Write on
`hex-releases` only), stored in the `release` environment as variable
`R2_ACCESS_KEY_ID` (the token id) and secret `R2_SECRET_ACCESS_KEY` (SHA-256
of the token value). `scripts/r2-put.sh` signs requests with curl's SigV4.
Immutable objects are never replaced with different bytes; feeds, the latest
pointer, and the appcast move last.

### Transition from Kit's hosting

Releases up to 2.3.1 compiled `https://downloads.hex.kitlangton.dev` (Kit's
personal `hex-releases` bucket; the website and installers used the `.com`
alias). Builds from 3a61146 on compile `https://downloads.hex.anoma.ly`.

- The new bucket was seeded with byte copies of every object the old one
  served (the 2.3.1 feeds and artifacts, installers, the appcast, DMGs, Sparkle
  ZIPs, and Setup.exe installers).
- Kit adds a redirect on his zone: `downloads.hex.kitlangton.dev/*` and
  `downloads.hex.kitlangton.com/*` → `https://downloads.hex.anoma.ly/$1`
  (302, path and query preserved). Sparkle and the Linux/Windows updaters
  (curl `--location`, HTTPS-only redirects, since the first updater) follow it,
  so installed apps read the new feeds without a bridge release. The redirect
  must stay up while old installs remain.
- Kit's bucket stays as it is, frozen at 2.3.1, with its
  `pub-089d681d41754031a4aefa7017d8c2fb.r2.dev` endpoint: builds 2.0.9–2.1.16
  read that URL directly (no redirect is possible there), update to 2.3.1,
  and reach the new origin from there.
- If the redirect is not possible, the fallback is one bridge release
  published to both buckets (Kit's credentials), after which the old bucket
  is frozen.
- Signing keys do not change. The Linux/Windows feed key is
  `HEX_RELEASE_SIGNING_KEY` (public key compiled in `src/self_update.rs`). The
  Sparkle EdDSA key must stay Kit's existing key (`SPARKLE_ED_PRIVATE_KEY`),
  because the first Anomaly macOS release also changes the Apple team, and
  Sparkle rejects an update that changes both.

## Secrets (`release` environment, limited to `main` and `app-v*`)

| Name | Use |
| --- | --- |
| `HEX_RELEASE_SIGNING_KEY` | ed25519 feed signing key (Linux, Windows) |
| `R2_SECRET_ACCESS_KEY`, var `R2_ACCESS_KEY_ID` | uploads to `hex-releases` |
| `AUR_SSH_PRIVATE_KEY` | pushes `hex-voice-bin` (key `github-actions-hex-aur`) |
| `AZURE_*` | Authenticode (docs/windows-signing.md) |
| `APPLE_*`, `SPARKLE_ED_PRIVATE_KEY` | macOS signing and appcast (docs/plans/macos-signing.md) |

## Linux packages

- **Direct install** (`curl -fsSL https://downloads.hex.anoma.ly/install-linux.sh | sh`):
  the user-local version layout; it self-updates from `linux-update.json` and
  the window offers "Update" (restart into the staged version).
- **AUR** `hex-voice-bin` (`packaging/aur`, `scripts/release-aur.sh`):
  `/usr/bin/hex`, the desktop entry, and the systemd user unit
  `/usr/lib/systemd/user/hex.service`, from the GitHub release's Linux
  executable. (`hex` is taken by the official hex-dump package, so the AUR
  name says what it is.) Maintained under Dax's AUR account.
- **Nix**: `nix/package.nix` builds from source; flake inputs update it.
- **Source** (`scripts/install-linux.sh`): the version layout with a
  `source-build` marker; the checkout updates it.

An executable outside the user-local layout that the user does not own (pacman's
`/usr/bin/hex`) or that lives in the Nix store never self-updates; Settings says
"Updated by your package manager". Source installs say "Updated from your
source checkout".
