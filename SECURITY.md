# Security Policy

## Supported versions

No version has been released yet. Only the latest commit on `main` is considered supported.

## Reporting a vulnerability

Please report privately using GitHub private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability** (<https://github.com/pzmi/site-preview/security/advisories/new>). Do not open a public issue for security problems.

## Scope

In scope:

- Path traversal outside the served root
- Symlink escape
- DNS rebinding
- `Host` / `Origin` check bypass
- Injection into served HTML

## Non-goals

- Site Preview is not a sandbox against other processes running as the same user.
- `--allow-remote` has no authentication and is insecure on untrusted networks; reports that amount to "remote access works without auth when `--allow-remote` is used" are by design.
- Behavior of third-party scripts or assets in the user's own site.
