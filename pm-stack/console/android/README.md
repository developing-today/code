# PM console for Android

A Trusted Web Activity: the app opens `https://pm.security.cab/console/`
in Chrome without browser chrome. It is the site, so it does everything
the site does, signs in the way the site does (Chrome's own session), and
never needs a release when the console changes. There is no code in the
module; the launcher activity comes from `androidbrowserhelper`.

## How the app and the site trust each other

- The app names the site in its manifest (`asset_statements`).
- The site names the app in `/.well-known/assetlinks.json`: package
  `today.developing.pmconsole`, signed by the certificate in
  [`signing-fingerprint.txt`](signing-fingerprint.txt). `pm-console`
  serves that file from `PM_CONSOLE_ANDROID_SHA256`.

If the two disagree the app still works, with a browser bar at the top.

## Build

```bash
gh workflow run pm-console-android.yml        # signed APK, attached to the release pm-console-android
bash pm-stack/console/android/build.sh        # locally: needs JAVA_HOME (17 or 21) and ANDROID_HOME
```

The workflow signs with the repository secrets
`PM_CONSOLE_ANDROID_KEYSTORE_B64` and
`PM_CONSOLE_ANDROID_KEYSTORE_PASSWORD`, and fails unless the APK's
certificate matches `signing-fingerprint.txt`. The keystore itself is on
the pm-stack host at `~/.local/state/pm-console/android/release.p12`.
Losing it means a new key, a new fingerprint and a reinstall; nothing
else depends on it.

## Install

Download `pm-console.apk` from the
[release](https://github.com/developing-today/code/releases/tag/pm-console-android)
on the phone and open it. Later builds install over it.
