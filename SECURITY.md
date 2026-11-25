# Security policy

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x | ✅ latest |
| < 0.1 | ❌ pre-release |

## Reporting a vulnerability

Please **do not** open a public issue for a security vulnerability.

- Use GitHub's *Security → Report a vulnerability* (private advisory) on
  `Connacri/android-cli-agent`, or
- contact the maintainer through GitHub if advisories are unavailable.

Include: affected version/commit, reproduction steps, impact, and any suggested fix.
You will get an acknowledgement as soon as possible; fixes for confirmed issues are
released through a tagged GitHub release.

## Scope highlights

- Destructive device operations are gated by explicit confirmation
  (see `docs/SECURITY.md`).
- No credentials, tokens or personal data are stored by the tool; the only state file
  contains previously connected Wi-Fi endpoints.
- The tool spawns only the official `adb` binary (argv arrays, no shell), with a timeout
  and `stdin` closed.
