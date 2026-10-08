# Security Policy

## Supported versions

tmxr is pre-1.0. Only the latest published release receives security fixes;
older releases are best-effort.

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.**

Email `mxaddict@kryptic.sh` with:

- Affected crate(s) and version(s)
- Description of the issue and impact
- Reproduction steps or proof-of-concept
- Disclosure timeline preference

Acknowledgment within 72 hours. Coordinated disclosure window is typically 30
days from acknowledgment, extendable for complex issues.

## Threat model

A tmxr server runs programs and accepts commands on your behalf, so anything
that can talk to its socket can run code as you. The design
([docs/plan/03-protocol.md](docs/plan/03-protocol.md)) restricts that to your
own user:

- Unix: the socket lives in a per-user directory created `0700` and verified
  (owner, mode, not a symlink), and every connection's peer uid is checked.
- Windows: the named pipe is created for the current user only.

Config files can bind keys to `run-shell`; treat a `config.toml` like a shell
script you are about to run.

## Dependencies

`cargo deny` and `cargo audit` run in CI on every push to `main` and every pull
request, checking RUSTSEC advisories along with the license and source rules in
[`deny.toml`](deny.toml). Dependabot opens grouped dependency PRs weekly (see
[`.github/dependabot.yml`](.github/dependabot.yml)).
