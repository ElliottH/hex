# Hex on the Anomaly release service

Contract between Hex's release workflow (publishing) and Hex's in-app updater
(client). The service and its rules are `anomalyco/release`
(`docs/protocol.md`, `docs/publishing.md`); this file pins Hex's names.

## Names

```ts
https://anoma.ly/release/hex/              production (dev: https://dev.anoma.ly/release/hex/)
├─ index.json                              download page and installers
└─ desktop/<channel>.json                  the manifest the app polls; channels stable, beta, dev
   └─ release.targets
      ├─ macos-arm64          Hex-<v>-macos-arm64.zip          ditto zip, root Hex.app (updater)
      ├─ macos-arm64-dmg      Hex-<v>-macos-arm64.dmg          notarized DMG (download page)
      ├─ windows-x64          Hex-<v>-windows-x64.exe          bare hex.exe (updater)
      ├─ windows-x64-setup    Hex-Setup-<v>-windows-x64.exe    Inno Setup installer (download page)
      ├─ windows-arm64        Hex-<v>-windows-arm64.exe
      ├─ windows-arm64-setup  Hex-Setup-<v>-windows-arm64.exe
      └─ linux-x64            hex-<v>-linux-x64                bare executable (updater, installer, AUR)
```

- App `hex`, artifact `desktop`. `stable` is the public channel; Release's
  `staging` runs publish `stable` to the dev service. `beta` and `dev` are
  registered for later use.
- Every executable and archive is signed before upload (Developer ID +
  notarization on macOS, Authenticode on Windows); the service stores bytes and
  their size and SHA-256, and manifests are not signed. Linux relies on HTTPS
  and the manifest's SHA-256.
- Versions are `X.Y.Z` on stable. The service refuses an equal or lower
  version unless the current one is revoked.

## Distributions

Recorded in every release; package-manager entries gain `version` only after
that job succeeds (the `distribution` action):

| Kind | Fields | Command |
| --- | --- | --- |
| `direct` | | (updates itself) |
| `aur` | `package: hex-bin` | `yay -Syu hex-bin` |
| `brew` | `cask: anomalyco/tap/hex` | `brew upgrade --cask anomalyco/tap/hex` |
| `nix` | `attribute: github:anomalyco/hex` | `nix profile upgrade hex` |

## Direct-install receipt

The installers write `install.json`
(`{"app":"hex","artifact":"desktop","channel":"stable","method":"direct","root":"<root>"}`)
at the root of the user-owned install, and the client self-updates only when
it is present and the root is writable by this user alone:

- Linux: `~/.local/share/voice-control/install.json` (`install-linux.sh`;
  `scripts/install-linux.sh` source builds write none and keep their
  `source-build` marker).
- Windows: `%APPDATA%\voice-control\install.json` (`install-windows.ps1` and
  the Setup.exe).
- macOS: the client decides from the bundle's location (a writable,
  non-translocated `Hex.app` is direct).

`HEX_RELEASE_BASE_URL` (HTTPS only) points a build at another service origin,
e.g. `https://dev.anoma.ly/release`, for dry runs.

## Old clients (2.3.1 and earlier)

`https://anoma.ly/hex/files/{linux-update.json,windows-update.json,appcast.xml}`
stay frozen at 2.3.1 in the `hex-production-files` bucket. At the first
release on the service, the appcast gains one informational item pointing at
https://anoma.ly/hex (see `docs/releasing.md`).
