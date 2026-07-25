# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's security-advisory
flow for this repository. Do not include API keys, OAuth tokens, project data,
or other secrets in a public issue.

## Desktop threat model

PAS is a local desktop application for programs the user chooses to run. The
Tauri IPC layer validates dataset identifiers, page sizes, filters, project
paths, AI providers, and HTTPS endpoints. AI requests are limited to two
concurrent calls and 20 starts per minute to contain accidental quota bursts.

PAS programs themselves are trusted local code. DuckDB SQL, `LIBNAME`, and
file-oriented DATA step features can access paths available to the PAS
process. The webview file-command allowlist is not a sandbox for a malicious
program.

## Content Security Policy

Production uses a restrictive default CSP and blocks renderer network access;
AI calls run in Rust. `script-src` currently includes `'unsafe-eval'` for the
bundled Monaco editor runtime. This weakens script-injection protection, so
the renderer is kept secret-free and all privileged actions remain behind
validated Tauri commands. Re-test Monaco and remove this exception when its
worker/runtime packaging no longer needs it.

## Secret storage

API keys and ChatGPT OAuth tokens are never stored in browser storage or
returned to the renderer after saving. They are encrypted with AES-256-GCM in
the application data directory. The encryption key is derived from the stable
app-data path, so this protects against casual inspection but is not equivalent
to an operating-system keychain: a local attacker with the files and binary can
recover them. OS keychain integration is tracked as future hardening.
