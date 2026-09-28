# Release

The Swift package is published by the
[`Release Swift (iOS)`](../.github/workflows/release-swift.yml) workflow. It is
the only release channel; there is no crates.io, Kotlin or React Native release.

## Versioning

- One semver tag per release, without a prefix: `0.1.0`.
- The tag is created by the workflow, not by hand. The workflow refuses to run
  when the tag already exists on `origin`.
- The tag version is independent of the crate version in
  `crates/solana-clearsign/Cargo.toml`.

## Trigger

Actions → **Release Swift (iOS)** → **Run workflow** → `version` (strict semver,
`^[0-9]+\.[0-9]+\.[0-9]+$`). Run it from `main` after the release content is
merged and CI is green.

## What the workflow does

1. Validates the version and checks out `main` with full history.
2. Fails if `refs/tags/<version>` already exists on `origin`.
3. Installs stable Rust with `aarch64-apple-ios`, `aarch64-apple-ios-sim` and
   `x86_64-apple-ios`.
4. Runs [`scripts/build-xcframework.sh`](../scripts/build-xcframework.sh), which
   builds every slice and regenerates the UniFFI Swift bindings.
5. Fails if the regenerated `bindings/swift/generated/` differs from the
   committed files. The committed bindings are the ones consumers compile, so
   they must match the binary.
6. Runs `swift test` against the local XCFramework.
7. Zips `target/ios/SolanaClearsignFFI.xcframework` into
   `Output/SolanaClearsignFFI.xcframework.zip` and computes
   `swift package compute-checksum`.
8. Rewrites [`Package.swift`](../Package.swift): `useLocalRustXCFramework`
   becomes `false`, the download URL gets the version, `checksum` gets the
   computed value. `swift package dump-package` confirms the binary target has
   the expected URL and checksum and no local path.
9. Commits the rewritten manifest as `Release <version>`, tags it and pushes
   only the tag. `main` is untouched.
10. Creates the GitHub Release with generated notes and attaches the zip.

## `main` versus a tag

- On `main`, `Package.swift` resolves `target/ios/SolanaClearsignFFI.xcframework`.
  Local development and CI build it with `scripts/build-xcframework.sh`.
- On a release tag, `Package.swift` resolves the GitHub Release asset. Consumers
  add the Git URL with an exact version:

```swift
.package(url: "https://github.com/llbartekll/solana-clear-signing-sdk.git", exact: "0.1.0")
```

- CI runs on branch pushes and pull requests only. The release commit exists
  only on the tag, and its manifest points at an asset that is uploaded after
  the tag is pushed.

## Regenerated bindings

`bindings/swift/generated/` is committed. After any change to the Rust FFI
surface, run `scripts/build-xcframework.sh` and commit the result; both CI and
the release workflow fail on drift. `uniffi` is pinned in `Cargo.lock`, so the
output is reproducible across machines.

## Recovery

- **Workflow failed before "Tag release"**: nothing was published. Fix the cause
  and re-run with the same version.
- **Tag pushed, release creation failed**: re-running refuses because the tag
  exists. Either create the release by hand for that tag and attach
  `Output/SolanaClearsignFFI.xcframework.zip` from the run artifacts, or delete
  the tag (`git push origin :refs/tags/<version>`) and re-run. Never reuse a
  version whose zip was already downloadable: the checksum in the tagged
  manifest is bound to those bytes.
- **Wrong contents shipped**: publish a new patch version. Do not overwrite a
  release asset in place.

## Limitations

- The XCFramework's macOS slice is built on the runner's host architecture.
  GitHub's `macos-15` runners are Apple Silicon, so the published macOS slice is
  `arm64` only. iOS device and simulator slices are complete.
- No pre-release or build-metadata versions; the version check accepts only
  `MAJOR.MINOR.PATCH`.
