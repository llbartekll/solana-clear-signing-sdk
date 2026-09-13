# TestFlight release

The demo is named **Clear Sign**, bundle ID `dev.clearsign.demo`, version `1.0`,
build `1`. It supports iPhone and iPad on iOS 16 or later. Release archives use
the public Devnet RPC and bundled fallback captures, without the local Alchemy
API key. No wallet connection, account, private key or funding is required.

## Archive and upload

1. Generate the native library with `scripts/build-xcframework.sh` if needed,
   then run `xcodegen generate --spec ios-demo/project.yml`.
2. Open `ios-demo/ClearsignDemo.xcodeproj`. In **Signing & Capabilities**, choose
   your Apple Developer Program team with automatic signing enabled. To preserve
   the team across project regeneration, set `DEVELOPMENT_TEAM` in the ignored
   `ios-demo/Config.local.xcconfig`.
3. Create an iOS app record in App Store Connect for the same bundle ID. If that
   identifier is unavailable, change `PRODUCT_BUNDLE_IDENTIFIER` in `project.yml`
   and regenerate before the first upload.
4. Select the **ClearsignDemo** scheme and **Any iOS Device (arm64)**, then
   **Product → Archive**. The shared scheme archives Release with dSYMs.
5. In Organizer, choose **Validate App**, then **Distribute App → App Store
   Connect → Upload**. Use the regular App Store Connect distribution option
   if you want external testers, rather than “TestFlight Internal Only”.
6. After processing, add the build to a TestFlight group. For internal testers,
   enable **automatic distribution** in the group. External testing requires
   beta review and contact/test information in App Store Connect.

Increase `CURRENT_PROJECT_VERSION` in `project.yml` for subsequent uploads and
regenerate the project, or let Organizer manage the build number during upload.
`MARKETING_VERSION` controls the user-visible version.

The generated `Info.plist` declares `ITSAppUsesNonExemptEncryption = false`.
The current read-only app uses system HTTPS and hashing; it does not implement
data confidentiality encryption. This declaration avoids the repeated export
compliance questionnaire, not Apple's processing or beta review. Reassess it
if encryption functionality is added.

## Suggested beta information

**Description:** Clear Sign demonstrates human-readable Solana subscription
permissions using IDL display metadata. Explore captured Devnet transactions,
inspect decoded amounts and account roles, and compare clear-signing output
with the original transaction data.

**What to test:** Review Enable Subscriptions, Recurring Delegation and Revoke
Delegation, then the four additional examples. Check readability, address
details, amount formatting and the technical details. Amounts without a resolved
scale intentionally remain marked as raw. Test the bundled examples offline.

**Review notes:** The app does not sign or submit transactions, hold funds, sell subscriptions or use
In-App Purchases. The term “subscriptions” refers to Solana program permissions.
No sign-in is required. Bundled examples remain available when Devnet RPC fails.

Provide your own feedback email and beta-review contact details in App Store
Connect; these are not part of the app binary.

## Privacy and assets

The app contains no analytics, advertising, tracking or application telemetry.
Its privacy manifest declares no tracking, collected data types or required-
reason API usage. RPC requests send transaction signatures and account addresses
to Solana's public Devnet endpoint, which also receives normal connection data
such as the IP address. Provider processing is separate from app telemetry;
review the provider's policy when completing App Store Connect privacy details.

The opaque 1024×1024 app icon is in `Resources/Assets.xcassets`. Recreate it with:

```sh
swift scripts/generate-app-icon.swift ios-demo/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png
```

Apple references: [uploading builds](https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds),
[internal distribution](https://developer.apple.com/help/app-store-connect/test-a-beta-version/add-internal-testers),
[external testing](https://developer.apple.com/help/app-store-connect/test-a-beta-version/invite-external-testers),
[export compliance](https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations).
