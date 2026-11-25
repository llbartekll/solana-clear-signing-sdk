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
merged and CI is green; the workflow refuses any other ref and any commit
without a successful CI run for its push to `main`.

## What the workflow does

The `build` job has read-only permissions; only the `publish` job, which runs no
project code, can write to the repository. Actions are pinned to commit SHAs and
no build cache is restored.

`build`:

1. Validates the version, requires `refs/heads/main`, and requires that the
   latest CI run for the push of that exact commit succeeded.
2. Checks out that commit and fails if `refs/tags/<version>` already exists on
   `origin`.
3. Installs stable Rust with `aarch64-apple-ios`, `aarch64-apple-ios-sim` and
   `x86_64-apple-ios`.
4. Runs [`scripts/build-xcframework.sh`](../scripts/build-xcframework.sh), which
   builds every slice and regenerates the UniFFI Swift bindings.
5. Fails if the build changed `Cargo.lock`, or if the regenerated
   `bindings/swift/generated/` differs from the committed files. The committed
   bindings are the ones consumers compile, so they must match the binary.
6. Runs `swift test` against the local XCFramework.
7. Zips `target/ios/SolanaClearsignFFI.xcframework` into
   `Output/SolanaClearsignFFI.xcframework.zip` and computes
   `swift package compute-checksum`.
8. Rewrites [`Package.swift`](../Package.swift): `useLocalRustXCFramework`
   becomes `false`, the download URL gets the version, `checksum` gets the
   computed value. `swift package dump-package` confirms the binary target has
   the expected URL and checksum and no local path.
9. Saves the zip and the rewritten manifest as the `release-files` run artifact.

`publish`:

1. Downloads `release-files`, checks the zip against the checksum, and fails if
   the tag, a release or a draft for the version already exists.
2. Commits the rewritten manifest as `Release <version>` and tags it locally.
3. Creates a draft release with generated notes and uploads the zip to it.
4. Pushes only the tag, then publishes the draft. `main` is untouched.
5. If a step fails before the release is public, deletes the draft and the tag
   it pushed.

## `main` versus a tag

- On `main`, `Package.swift` resolves `target/ios/SolanaClearsignFFI.xcframework`.
  Local development and CI build it with `scripts/build-xcframework.sh`.
- On a release tag, `Package.swift` resolves the GitHub Release asset. Consumers
  add the Git URL with an exact version:

```swift
.package(url: "https://github.com/llbartekll/solana-clear-signing-sdk.git", exact: "0.1.0")
```

- CI runs on branch pushes and pull requests only. The release commit exists
  only on the tag, and its manifest points at an asset that becomes downloadable
  when the draft release is published, right after the tag is pushed.

## Regenerated bindings

`bindings/swift/generated/` is committed. After any change to the Rust FFI
surface, run `scripts/build-xcframework.sh` and commit the result; both CI and
the release workflow fail on drift. `uniffi` is pinned in `Cargo.lock`, so the
output is reproducible across machines.

## Recovery

- **`build` failed**: nothing was published. Fix the cause and dispatch again
  with the same version.
- **`publish` failed and rolled back**: no tag or draft is left. Use **Re-run
  failed jobs** to publish the same saved files, or dispatch again.
- **`publish` was interrupted before it could roll back** (for example the
  runner was lost): check for a tag and a draft named `<version>`. If the
  release is not public, delete both (`gh release delete <version>`,
  `git push origin :refs/tags/<version>`) and use **Re-run failed jobs**.
- **The tag is public and must stay**: download `release-files` from the run and
  attach that exact zip to the release for the tag. Never attach a rebuilt zip:
  the checksum in the tagged manifest is bound to the original bytes. Never
  reuse a version whose zip was already downloadable.
- **Wrong contents shipped**: publish a new patch version. Do not overwrite a
  release asset in place.

## Limitations

- The XCFramework's macOS slice is built on the runner's host architecture.
  GitHub's `macos-15` runners are Apple Silicon, so the published macOS slice is
  `arm64` only. iOS device and simulator slices are complete.
- No pre-release or build-metadata versions; the version check accepts only
  `MAJOR.MINOR.PATCH`.
