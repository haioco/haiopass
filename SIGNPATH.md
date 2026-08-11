# SignPath code-signing setup (free for open-source)

This repo uses [SignPath Foundation](https://signpath.org) to Authenticode-sign
the Windows installer and executable on every release, free of charge because
the repo is public.

## What gets signed

- `*-setup.exe` (NSIS installer)
- `HaioBypass.exe` (the Tauri binary embedded in the installer is signed
  inside the SignPath signing step; the NSIS installer itself is also signed)

Non-Windows artifacts (`.deb`, `.AppImage`, `.dmg`) are uploaded directly to
the GitHub release without code signing — Linux and macOS don't use
Authenticode.

## One-time manual setup

You only do these steps once.

### 1. Apply to SignPath Foundation

Go to <https://signpath.org/oss/apply> and submit `haioco/haiopass` for the
Open Source program. You'll need:

- A public repo (already public — done)
- A valid `LICENSE` file in the repo root (add one if missing)
- Maintainer contact email (use your haio.ir address)

SignPath reviews the application and provisions an organization for you.

### 2. Install the SignPath GitHub App

Visit <https://github.com/apps/signpath> and install the app on the `haioco`
organization, granting access to the `haiopass` repository. This lets SignPath
verify that builds originated from this repo's workflows.

### 3. Create a SignPath project + signing policy

In the SignPath web UI:

1. Create a project named **`haiobypass`** (this matches `project-slug` in
   `release.yml`).
2. Create a signing policy named **`release-signing`** (matches
   `signing-policy-slug`).
3. Link the *GitHub.com* Trusted Build System to the project.
4. Under the project's Artifact Configuration, add a `<zip-file>` root
   element (because `actions/upload-artifact@v4` produces a ZIP — this is
   the default). Configure signing for these file types inside the ZIP:
   - `*.exe` — Authenticode
   - `*.msi` — Authenticode (optional, only if you enable MSI in `tauri.conf.json`)
5. Generate an **API token** for a submitter user.

### 4. Add GitHub secrets and a variable

In the GitHub repo: **Settings → Secrets and variables → Actions**

Add these **repository secrets**:

| Name | Value |
|---|---|
| `SIGNPATH_API_TOKEN` | The API token from SignPath |
| `SIGNPATH_ORGANIZATION_ID` | The organization ID shown in SignPath's organization settings |
| `TAURI_SIGNING_PRIVATE_KEY` | Output of `cargo tauri signer generate -w ~/.tauri/haio.key` (private key for updater manifests; unrelated to SignPath cert) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password you chose when generating the updater key |

Add this **repository variable** (Secrets and variables → *Variables* tab):

| Name | Value |
|---|---|
| `SIGNPATH_ENABLED` | `true` |

Setting `SIGNPATH_ENABLED=true` makes the workflow submit the Windows build
to SignPath. Without it (or set to `false`), the workflow uploads the
**unsigned** Windows build instead — useful for testing the build pipeline
before SignPath is provisioned.

### 5. Put the updater public key in `tauri.conf.json`

Generate once:

```bash
cargo tauri signer generate \
  -w ~/.tauri/haio.key \
  -p "your-strong-password"
```

It prints a public key like
`dW50cnVzdGVkIGNvbW1l...`. Paste it into the `pubkey` field of the `updater`
plugin block in `src-tauri/tauri.conf.json` (replace the placeholder).

The **private** key goes to the GitHub secret `TAURI_SIGNING_PRIVATE_KEY`,
the **password** to `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit the
private key.

## How a release happens

1. You push a tag, e.g. `git tag v1.0.7 && git push origin v1.0.7`.
2. The `Release` workflow builds for Linux, Windows, macOS.
3. The Windows build is uploaded as a **workflow artifact**.
4. The `signpath/github-action-submit-signing-request@v2` action submits this
   artifact to SignPath. SignPath's GitHub connector verifies:
   - The workflow actually ran on GitHub-hosted runners.
   - The artifact was produced by this repo's workflow run.
   - The source/build policy in `.signpath/policies/haiobypass/release-signing.yml`
     is satisfied.
5. SignPath signs the `*.exe`/`*.msi` files inside the artifact ZIP using
   their foundation-issued certificate, returns the signed ZIP.
6. The workflow extracts the signed files and uploads them to the draft
   GitHub release.
7. Non-Windows artifacts (`*.deb`, `*.AppImage`, `*.dmg`) are uploaded
   directly during the build job.
8. After all matrix jobs finish, the `publish` job flips the release from
   draft to published.

## Submitting the first signed build to Microsoft

Even after SignPath signs it, Windows Defender SmartScreen will still warn
on the first few downloads because the certificate is new and has no
reputation. To accelerate this:

1. Build and sign version `1.0.7`.
2. Download the signed `.exe` installer.
3. Submit it at <https://www.microsoft.com/en-us/wdsi/filesubmission>
   as a false-positive report. Mention the SignPath Foundation cert.
4. Microsoft reviews and whitelists the certificate reputation within a few
   days. Subsequent builds (signed with the same cert) won't trigger
   SmartScreen warnings.

## Troubleshooting

**Workflow uploaded the unsigned build instead of signing it.**
Check that the repository variable `SIGNPATH_ENABLED` is set to exactly `true`
(spelled the same as in the workflow `if`).

**SignPath returns "artifact not found".**
The `signpath/github-action-submit-signing-request` action requires the
artifact to be uploaded in the *same* workflow run. This is why the SignPath
submit step is inside the Windows matrix job, not a separate job.

**"origin verification failed" on SignPath.**
Make sure you installed the SignPath GitHub App on the `haioco` organization
and granted it access to the `haiopass` repo. Re-install if needed.

**Build fails on the SignPath step with branch ruleset errors.**
Your branch rulesets in GitHub don't satisfy the policy in
`.signpath/policies/haiobypass/release-signing.yml`. Either tighten the
branch ruleset on the `main` branch (recommended) or relax the policy file.
