# Security policy

## Supported versions

Only the [latest release](https://github.com/ManyGlue0/Holster/releases/latest) gets security fixes. Holster updates itself from Settings → Updates.

## Reporting a vulnerability

Please report it privately: open the repository's **Security** tab and choose **Report a vulnerability**. Don't open a public issue for it.

Include what you found, how to reproduce it, and what an attacker could do with it. You'll get an answer within a week, and credit in the release notes if you'd like it.

## What's in scope

- Holster's local API (`127.0.0.1` on a random port): reaching it from a website or another machine, DNS rebinding, reading files outside an app's preview folder.
- The updater: installing anything not signed with Holster's key, or a downgrade.
- App manifests and notifications: anything that makes Holster run, open or show something the manifest didn't allow.
- The `hello-holster` example and the `holster-app` skill, where they teach an unsafe pattern.

Out of scope: anything that already requires running code as the same Windows user. That user can edit Holster's manifests directly, and they're trusted to.
