# Release checklist

Use this checklist for every release PR. Record the decisions in the release
PR even when the answer is "not affected" or "no mobile release required."
The goal is to know which existing installations were tested and whether users
must update one app, both peers, or neither.

## What the release workflow publishes

Merging an ordinary change into `main` updates the release-please PR. Merging
the release-please PR creates the version tag and GitHub release, then the
release workflow handles these artifacts:

| App | Target | Distribution when a release is created |
| --- | --- | --- |
| macOS sender | `OpenSidecarMac` | Notarized universal DMG on GitHub Releases and its Sparkle feed |
| macOS receiver | `OpenSidecarMacReceiver` | Notarized universal DMG on GitHub Releases and its separate Sparkle feed |
| iPhone and iPad receiver | `OpenSidecariOS` | One universal build uploaded to TestFlight |

Both macOS apps are rebuilt and published on every release. The iPhone/iPad
build is uploaded to TestFlight on every release, but publishing it to the App
Store is a separate manual decision. A TestFlight upload does not update App
Store users.

## Release impact record

Copy this section into the release PR and fill it in before creating the tag.
"Changed" means its source or assets changed. "Affected" includes behavior
changed through shared code or a peer's protocol behavior. "Required" means
users need that release to receive the fix or remain compatible.

| App | Changed? | Affected? | Release required? | Reason |
| --- | --- | --- | --- | --- |
| macOS sender | | | | |
| macOS receiver | | | | |
| iPhone receiver | | | | |
| iPad receiver | | | | |

- App Store production release: [ ] required [ ] recommended [ ] not required
- Reason for the App Store decision:
- Minimum useful update combination, such as "sender only" or "both peers":
- User-visible behavior or migration to mention in release notes:

## Compatibility and update order

- [ ] State whether the wire format, discovery, persistence, permissions, or
      saved display identity changed.
- [ ] Record `WireProtocol.version` and `WireProtocol.minSupportedPeer` before
      and after the change.
- [ ] Confirm that new fields and messages are additive and safely ignored by
      released peers, or document the deliberate incompatibility.
- [ ] Test or explicitly reason through every relevant pairing:
  - [ ] new sender with released iPhone/iPad receiver
  - [ ] released sender with new iPhone/iPad receiver
  - [ ] new sender with released macOS receiver
  - [ ] released sender with new macOS receiver
  - [ ] new sender with new receiver
- [ ] State whether a forced update is required. If yes, explain why graceful
      fallback is impossible and which versions are rejected.
- [ ] When peers must be released together, publish and verify the receiving
      app first. Do not enforce its new minimum version until it is available
      to users.
- [ ] Confirm that upgrading and downgrading preserve existing permissions,
      settings, display arrangement, and pairing state where applicable.

Prefer capability negotiation and fallback over forced updates. A protocol
version bump alone does not require rejecting an older peer; change the minimum
supported peer only when continuing would be incorrect or unsafe.

## Build and automated verification

- [ ] Generate the project from the committed configuration:

  ```sh
  xcodegen generate
  ```

- [ ] Run the macOS unit suite and sender compile check:

  ```sh
  xcodebuild test \
    -project OpenSidecar.xcodeproj \
    -scheme OpenSidecarMac \
    -destination 'platform=macOS' \
    CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY=""
  ```

- [ ] Build the macOS receiver at its macOS 12 deployment floor.
- [ ] Build the generic iOS/iPadOS target. This is required when `Shared/`,
      `iOS/`, protocol code, assets, or project settings changed.
- [ ] For a release candidate, build both macOS apps in Release configuration
      and verify their executables contain `arm64` and `x86_64` with
      `lipo -info` or `file`.
- [ ] Confirm required GitHub checks pass on the final commit.
- [ ] Run `git diff --check` and confirm the working tree is clean.

## End-to-end verification

Choose the rows affected by the release and record the devices, OS versions,
resolution, refresh rate, transport, and app versions used.

- [ ] Extend mode connects, displays changing content, and accepts the cursor.
- [ ] Mirror mode connects and displays changing content.
- [ ] Best, Balanced, and Fast still select valid stream configurations.
- [ ] Rotation or receiver resolution changes reconfigure without stale video.
- [ ] Disconnect, reconnect, receiver quit/reopen, and sleep/wake recover without
      requiring a logout or computer restart.
- [ ] Wi-Fi works with useful frame latency and no unbounded queue growth.
- [ ] iPhone/iPad USB works if sender, transport, framing, or shared receiver
      code changed.
- [ ] Mac-to-Mac Thunderbolt Bridge or Ethernet works if connection migration,
      framing, or transport code changed.
- [ ] Pulling a cable falls back or fails clearly according to the documented
      behavior.
- [ ] The oldest supported macOS receiver launches on Intel hardware when
      receiver or shared code changed.
- [ ] Existing App Store iPhone/iPad builds remain usable when no production
      mobile release is planned.

For performance-sensitive changes, compare the same scene before and after.
Record resolution and frame-rate selection, p50/p95 latency, encode time,
delivered FPS, dropped frames, cursor update rate/loss, and subjective behavior.
A clean average does not excuse severe tail latency or recovery failures.

## Packaging and publication

- [ ] Release notes say which apps users need to update and whether mixed
      versions remain supported.
- [ ] Version and build numbers are correct in all generated artifacts.
- [ ] The GitHub release contains both `OpenDisplay.dmg` and
      `OpenDisplayReceiver.dmg`.
- [ ] Both DMGs and contained apps pass signature, notarization, and Gatekeeper
      checks.
- [ ] The sender and receiver Sparkle appcasts point to the correct DMGs and a
      released installation can update through each feed.
- [ ] The TestFlight upload finishes processing and launches on both an iPhone
      and an iPad when the mobile app is affected.
- [ ] If an App Store production release is required, update its notes and
      metadata, submit it after the TestFlight smoke test, and record whether
      release is manual or automatic after approval.
- [ ] Publish user-facing compatibility limits, required update order, known
      issues, and any one-time action users must take.

## After release

- [ ] Install through Sparkle on both macOS apps rather than testing only the
      downloaded DMGs.
- [ ] Repeat one representative new/new connection and one promised mixed-version
      connection using the published binaries.
- [ ] Check logs for connection loops, decode failures, encoder rejection,
      update-feed errors, or permission regressions.
- [ ] Confirm the download page, release assets, Sparkle feeds, and App Store or
      TestFlight version show the intended release.
- [ ] Keep the previous signed artifacts available as the rollback path and
      document any data or protocol state that prevents a safe downgrade.
