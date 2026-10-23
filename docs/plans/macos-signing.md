# macOS signing under Anomaly

Move Hex's macOS Developer ID signing and notarization from Kit's personal team
(`QC99C9JE59`, local keychain and `HEX_NOTARY_PROFILE` on his Mac) to Anomaly's
team in GitHub Actions, reusing the credentials OpenCode already uses.

## What OpenCode does

Desktop and CLI (`anomalyco/opencode`, `.github/workflows/publish.yml` on `dev`):

```ts
team         // Anomaly Innovations, Inc. (5NZ4Q7NXJ4); confirmed from the signed
             // opencode-darwin-arm64 npm binary, not written in the repo
runners      // macos-26 (arm64) and macos-26-intel, runner-default Xcode
certificate  // apple-actions/import-codesign-certs@8f3fb60 (v2.0.0), keychain "build"
             // p12-file-base64: APPLE_CERTIFICATE, p12-password: APPLE_CERTIFICATE_PASSWORD
             // no KEYCHAIN_PASSWORD secret: the action generates one
identity     // security find-identity -v -p codesigning build.keychain
             // | first "Developer ID Application: …"
cli          // job sign-cli-macos: codesign --force --timestamp --options runtime
             // --entitlements packages/cli/script/entitlements.plist (JIT, unsigned
             // memory, dyld env, no library validation: Bun needs them)
desktop      // electron-builder (packages/desktop/electron-builder.config.ts):
             // hardenedRuntime, entitlements resources/entitlements.plist
             // (allow-jit, audio-input), notarize: true, targets dmg + zip
notary       // App Store Connect API key through notarytool (electron-builder):
             // APPLE_API_KEY_PATH secret holds the .p8 *contents*, written to
             // $RUNNER_TEMP/apple-api-key.p8; APPLE_API_KEY holds the key ID;
             // APPLE_API_ISSUER the issuer ID
updates      // electron-updater generic feed; no separate update-signing key
             // (Squirrel.Mac trusts the code signature)
windows      // Azure Trusted Signing (AZURE_* secrets), separate from Apple
```

The repo also has `APPLE_ID` and `APPLE_PASSWORD` secrets, but no workflow
reads them: Apple ID plus app-specific password is not the current method.

iOS (`anomalyco/opencode-ios`, private): Xcode Cloud archives and ships to
TestFlight on every push to `main`; signing is automatic (`project.yml`:
`DEVELOPMENT_TEAM: 5NZ4Q7NXJ4`, `CODE_SIGN_STYLE: Automatic`), no fastlane or
match. Manual fallbacks use the `asc` CLI and `scripts/testflight-upload.sh` with
an App Store Connect key from 1Password (`docs/releasing.md`). Reusable for Hex:
the team, and an App Store Connect API key. A team key with the Developer role
can notarize Mac apps, so Hex can use the same key as OpenCode desktop.

## What Hex does today

```ts
scripts/build-app.sh       // cargo build → Hex.app; signs Sparkle, Moonshine and
                           // ONNX dylibs, HexMicrophone.driver, then the app with
                           // --options runtime and app/VoiceControl.entitlements
                           // (audio-input, apple-events; not sandboxed)
                           // HEX_TEST_BUILD=1: ad-hoc "Hex Test.app", no Sparkle
scripts/macos-signing.sh   // identity must be "Developer ID Application: … (TEAM)"
scripts/release-app.sh     // clean pushed tree, build newer than the live appcast,
                           // notarize/staple app and DMG (scripts/notarize-app.sh),
                           // Sparkle ZIP, generate_appcast with the EdDSA key from
                           // Kit's login keychain; "publish" uploads to R2
scripts/validate-app.sh    // bundle id com.kitlangton.hex2, feed
                           // downloads.hex.kitlangton.dev, SUPublicEDKey, team
test-builds.yml            // per-push ad-hoc Hex Test DMG (unsigned)
```

## Plan

1. **Release build in CI (implemented).** `.github/workflows/build-macos.yml`
   (`workflow_call`, environment `release`) runs `scripts/release-app.sh prepare`
   on `macos-26` with team `5NZ4Q7NXJ4`: build, Developer ID signing,
   notarization and stapling of the app and DMG (`scripts/notarize-app.sh`), the
   Sparkle ZIP (root `HEX.app`), and the appcast at `<download-base>/appcast.xml`
   (the live appcast when a staging prefix has none) extended with an item
   EdDSA-signed with `SPARKLE_ED_PRIVATE_KEY` (`HEX_SPARKLE_KEY_FILE`). It then
   checks staples, Gatekeeper, `validate-app.sh` on both bundles, identical
   payloads, the item's signature against the app's `SUPublicEDKey`, and a
   15-second launch (`scripts/smoke-macos-app.sh`). It uploads one artifact,
   `hex-macos-arm64` (DMG, ZIP, `appcast.xml`); the Release workflow publishes
   archives first and the appcast last. Missing secrets fail the job.
   `release-app.sh` keeps working locally with `HEX_NOTARY_PROFILE` and Kit's
   keychain key.
2. **Signed candidates.** `.github/workflows/macos-signed.yml` (manual) builds,
   signs, notarizes, validates, and launches a DMG without touching any appcast;
   it skips when the Apple secrets are absent.
3. **Sign the test builds.** Once the secrets exist, `test-builds.yml` can sign
   `Hex Test.app` with the same Developer ID (needs `build-app.sh` to accept a
   team with `HEX_TEST_BUILD=1`). The designated requirement then stays stable
   across builds, so testers grant permissions once instead of every build, and
   the `xattr` quarantine step goes away.
4. **Keep the Sparkle key.** Kit exports the existing private key
   (`generate_keys -x`) into `SPARKLE_ED_PRIVATE_KEY`. The Apple team changes in
   the same release, so the EdDSA key cannot rotate there.
5. **First Anomaly-signed release.** Ship release notes that explain the
   permission re-grant, validate updating from the current Kit-signed release on
   a real Mac, then publish. Update AGENTS.md's signing section,
   `test-app-identity.sh` fixtures if needed, and the Homebrew cask.

## Feed host

Downloads move to `https://anoma.ly/hex/files` (Anomaly's
`hex-production-files` R2 bucket). The bucket is seeded with byte copies of Kit's, and
`downloads.hex.kitlangton.{dev,com}/*` redirects there; Sparkle follows the
redirect, so installed apps with the old `SUFeedURL` read the new appcast and
updates carry the new `SUFeedURL`. The bundle ID stays `com.kitlangton.hex2`.

## Secrets for `anomalyco/hex`

Add to the `release` environment (already limited to `main` and `app-v*` tags;
it holds `HEX_RELEASE_SIGNING_KEY`). Names mirror OpenCode; GitHub cannot copy
secret values, so an admin re-enters them from the source (1Password):

```ts
APPLE_CERTIFICATE           // base64 .p12 of the Anomaly "Developer ID Application" cert + key
APPLE_CERTIFICATE_PASSWORD  // the .p12 password
APPLE_API_KEY_PATH          // App Store Connect API key .p8 contents (OpenCode's naming)
APPLE_API_KEY               // that key's ID
APPLE_API_ISSUER            // the App Store Connect issuer ID
SPARKLE_ED_PRIVATE_KEY      // Hex's existing Sparkle EdDSA private key (from Kit, generate_keys -x)
```

`APPLE_ID`, `APPLE_PASSWORD`, and `KEYCHAIN_PASSWORD` are not needed.

## Implications of changing the team

- **Bundle ID stays.** A Developer ID team change does not require a new
  bundle identifier; `com.kitlangton.hex2` can be signed by Anomaly.
- **Sparkle updates keep working if the EdDSA key is kept.** Sparkle allows an
  update that changes the Apple code-signing certificate or the EdDSA key, never
  both at once. Hex sets `SUVerifyUpdateBeforeExtraction`, so the EdDSA key
  must not rotate in the same release. Validate the Kit → Anomaly update on a
  real Mac before publishing.
- **Users must re-grant permissions.** TCC records each grant against the app's
  designated requirement, which names the signing team. After the first
  Anomaly-signed update, Microphone, Input Monitoring, Accessibility,
  Automation, Screen and System Audio Recording (Meetings) stop matching; old
  entries can remain listed but inert, so users may need to remove and re-add
  Hex. Onboarding's permission health gate must show this; release notes must
  say so.
- **Launch at login.** `SMAppService.mainAppService` registrations are tracked
  by Background Task Management with the team; expect the item to need
  re-enabling or approval after the switch. Hex polls macOS for the status
  (no stored flag), so Settings shows the truth; verify on a real Mac.
- **HexMicrophone.driver.** The bundled copy is re-signed by Anomaly; an
  installed copy in `/Library/Audio/Plug-Ins/HAL` keeps Kit's signature until
  the in-app Update reinstalls it.
- **Keychain items and app groups.** Hex stores none, so nothing else breaks.

## Bundle ID: later

Changing the bundle ID (for example to an `ai.opencode` or `ly.anoma` prefix)
makes macOS treat Hex as a new app: settings domain, login item, and
permissions all reset, and Sparkle discovery from existing installs needs the
filename workaround already used for 2.0.x. It is not needed for signing;
doing it together with the team change multiplies the risk.
