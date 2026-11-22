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
gh workflow run pm-console-android.yml        # builds the APK in CI
bash pm-stack/console/android/build.sh        # locally: needs JAVA_HOME (17 or 21) and ANDROID_HOME
bash pm-stack/console/android/sign.sh app-release-unsigned.apk   # sign a CI build with the key on this machine
```

The signing key is on the pm-stack host at
`~/.local/state/pm-console/android/release.p12`, and it stays there. The
workflow builds an unsigned APK and keeps it as the artifact
`pm-console-apk-unsigned`; `sign.sh` on the host signs it, checks the
certificate against `signing-fingerprint.txt`, and the signed
`pm-console.apk` is uploaded to the release:

```bash
gh run download <run-id> -n pm-console-apk-unsigned -D /tmp/pm-apk
nix shell nixpkgs#apksigner -c bash pm-stack/console/android/sign.sh /tmp/pm-apk/app-release-unsigned.apk
gh release upload pm-console-android /tmp/pm-apk/pm-console.apk --clobber
```

If the repository secrets `PM_CONSOLE_ANDROID_KEYSTORE_B64` and
`PM_CONSOLE_ANDROID_KEYSTORE_PASSWORD` are ever set, the workflow signs
and publishes by itself instead, and fails unless the certificate
matches. Losing the key means a new key, a new fingerprint and a
reinstall; nothing else depends on it.

## Install

On the phone, open `https://pm.security.cab/console/`, sign in, and tap
"Android app" in the header. That downloads the signed build from the
host (`/console/pm-console.apk`, owner only); open it to install. Later
builds install over it.

The console serves whatever signed build is at
`~/.local/state/pm-console/android/pm-console.apk` on the host, so
putting a new build in place is the whole release:

```bash
install -m 644 /tmp/pm-apk/pm-console.apk ~/.local/state/pm-console/android/pm-console.apk
```

The same file can also be attached to the GitHub release
`pm-console-android` for a copy outside the host.
