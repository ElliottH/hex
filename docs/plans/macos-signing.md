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

1. **CI signing path (implemented, opt-in).** `.github/workflows/macos-signed.yml`
   (manual dispatch) builds the release-identity `Hex.app`, signs with team
   `5NZ4Q7NXJ4`, notarizes and staples the app and DMG through
   `scripts/notarize-app.sh`, runs `scripts/validate-app.sh`, and uploads the DMG
   as a 14-day artifact. Without the secrets it skips, with a note in the run
   summary. It publishes nothing and signs no appcast. `hex_notarize` in
   `scripts/macos-signing.sh` uses `HEX_NOTARY_PROFILE` when set (local releases
   unchanged), else the API key (`HEX_NOTARY_KEY`, `HEX_NOTARY_KEY_ID`,
   `HEX_NOTARY_ISSUER`).
2. **Sign the test builds.** Once the secrets exist, `test-builds.yml` can sign
   `Hex Test.app` with the same Developer ID (needs `build-app.sh` to accept a
   team with `HEX_TEST_BUILD=1`). The designated requirement then stays stable
   across builds, so testers grant permissions once instead of every build, and
   the `xattr` quarantine step goes away. Notarization is optional there.
3. **Move the Sparkle key, not change it.** Keep `SUPublicEDKey`. Kit exports the
   private key (`generate_keys -x`) into a `SPARKLE_ED_PRIVATE_KEY` secret in the
   `release` environment; `release-app.sh` then reads the key from a file in CI
   instead of the keychain. Until then, signed CI DMGs are candidates and Kit
   signs the appcast locally.
4. **First Anomaly-signed release.** Run `release-app.sh` (or a CI port of its
   prepare step) with `VOICE_CONTROL_TEAM_ID=5NZ4Q7NXJ4`, ship release notes that
   explain the permission re-grant, validate updating from the current
   Kit-signed release, then publish as usual. Update AGENTS.md's signing
   section, `test-app-identity.sh` fixtures if needed, and the Homebrew cask.
5. **Later, separately: identity and hosts.** Keep `com.kitlangton.hex2` and
   `downloads.hex.kitlangton.dev` for now (see below).

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
SPARKLE_ED_PRIVATE_KEY      // step 3 only: Hex's existing Sparkle EdDSA private key (from Kit)
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

## Bundle ID and feed host: later

Changing the bundle ID (for example to an `ai.opencode` or `ly.anoma` prefix)
makes macOS treat Hex as a new app: settings domain, login item, and
permissions all reset, and Sparkle discovery from existing installs needs the
filename workaround already used for 2.0.x. The feed host can move with a
redirect, but `SUFeedURL` is baked into installed apps and
`validate-app.sh`. Neither is needed for signing, and doing them together with
the team change multiplies the risk; do the team change first and revisit the
identity and domains as a separate migration.
