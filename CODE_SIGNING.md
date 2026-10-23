# Code signing policy

**Current status: unsigned.** Releases are built by GitHub Actions and published
with SHA-256 checksums plus Sigstore build-provenance attestations; see
[Verifying a release](#verifying-a-release) below. Free code signing is planned
via the [SignPath Foundation](https://signpath.org/) open-source program; this
document is the project's signing policy and records the rules that will apply
while a certificate is active in this project's name.

Free code signing provided by [SignPath.io](https://signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/).

## Roles

This is a single-maintainer project. The same person holds every role, which is
stated here explicitly for review purposes:

| Role | Holder |
| :--- | :--- |
| Authors / committers (write access) | [`@DefinitlyEvil`](https://github.com/DefinitlyEvil) |
| Reviewers (review before release) | [`@DefinitlyEvil`](https://github.com/DefinitlyEvil) |
| Approvers (authorize each signing request) | [`@DefinitlyEvil`](https://github.com/DefinitlyEvil) |

Releases are cut only from `main` by pushing a `v*` tag. The release workflow
requires a manual approval before any artifact is signed.

## Privacy

This program will not transfer any information to other networked systems unless
specifically requested by the user or the person installing or operating it.

There is no telemetry, no update check, no analytics and no network code in the
application at all. The scanner only opens handles to files and directories on
the local volume the user selects, read-only.

## What gets signed

Only artifacts built by the workflow in
[`.github/workflows/release.yml`](.github/workflows/release.yml) from a commit on
`main` in this repository, on GitHub-hosted Windows runners:

- `disk-usage-inspector.exe` (GUI)
- `duiscan.exe` (console benchmark harness)

No third-party binaries are bundled, and nothing is signed that was not built
from this source tree. The release archive also contains `README.md`, `LICENSE`,
`NOTICE`, this policy and `SHA256SUMS.txt`.

## Signing procedure

1. Push an annotated tag `vX.Y.Z` to `main`.
2. The workflow runs the full gate (`fmt`, `clippy -D warnings`, tests, and the
   read-only API grep) and builds with the pinned GNU toolchain.
3. The unsigned artifact is uploaded, then submitted to SignPath.io for signing
   (manual approval by the Approver).
4. The signed archive and checksums are attached to the GitHub release.

The private key never exists on a maintainer machine: it lives in SignPath's HSM
and is used only by the signing service.

## Verifying a release

Every release ships `SHA256SUMS.txt`. Verify the download with:

```powershell
Get-FileHash .\disk-usage-inspector-vX.Y.Z-win64.zip -Algorithm SHA256
```

Verify that the archive was built by this repository's workflow:

```powershell
gh attestation verify .\disk-usage-inspector-vX.Y.Z-win64.zip --owner DefinitlyEvil
```

Verify an Authenticode signature, once signing is active:

```powershell
Get-AuthenticodeSignature .\disk-usage-inspector.exe | Format-List
```

The expected signer will be **SignPath Foundation**. Until then the signature is
`NotSigned` and Windows SmartScreen may show an "unrecognized app" prompt for a
freshly published build; that is expected, and verifying the checksum or the
attestation is the strongest available check.

## History

| Date | Change |
| :--- | :--- |
| 2026-10-24 | Policy created; releases published unsigned with checksums and build provenance. SignPath Foundation application planned after the first public release. |
