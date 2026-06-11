# Security Policy

## Reporting a vulnerability

Please report security issues privately rather than opening a public
issue. Use GitHub's **["Report a vulnerability"](../../security/advisories/new)**
(Security → Advisories) so the report stays confidential until a fix is
ready.

Include, where you can:

- the affected version / commit,
- a minimal pipeline file or command that triggers the issue,
- what you expected versus what happened.

We aim to acknowledge a report within a few days and to agree a
disclosure timeline with you.

## Scope

Pipewright parses untrusted CI/CD configuration files and can execute
pipelines locally in Docker. Reports we especially care about:

- **Parser** — crashes, panics, or unbounded resource use on a crafted
  config (the parsers are written to return errors, never to panic, on
  malformed input).
- **`pipewright run`** — the local runner bind-mounts the pipeline's
  directory into a container. Treat a pipeline you didn't write as
  untrusted code: its commands run with your Docker permissions. The
  mount is **read-only by default**; write access is opt-in (`--rw-copy`
  for a throwaway copy, `--rw` for in-place, which the UI confirms
  first). A way for a default (read-only) run to modify the host
  directory, or any other escape beyond the documented `--rw` behaviour,
  is in scope.
- **Migration / emit** — producing output that silently drops or
  corrupts security-relevant fields (conditions, secrets handling).

## Supported versions

This is pre-1.0 software; only the latest release receives security
fixes.
