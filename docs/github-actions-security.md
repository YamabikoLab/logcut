# GitHub Actions security maintenance

All third-party actions under `.github/workflows/` are pinned to full 40-character commit SHAs. The trailing comment records the upstream version represented by each SHA.

Dependabot checks GitHub Actions weekly through `.github/dependabot.yml`. When reviewing an update, confirm that the proposed SHA belongs to the version named in the comment, review the upstream release notes, and keep the full SHA plus version comment together.

The Rust toolchain action is also SHA-pinned. Validation and release builds intentionally use the `stable` toolchain channel so they continue to receive Rust fixes, while the separate Rust 1.70 job preserves the minimum supported compiler check. Release reproducibility therefore covers the workflow action implementation and locked Cargo dependencies, but not an immutable Rust compiler version.

Release provenance is generated for both published tarballs and `SHA256SUMS` before the GitHub Release is created. The attestation permissions are restricted to the release job.

After downloading a release artifact, verify its provenance with GitHub CLI:

```bash
gh attestation verify logcut-v0.1.11-x86_64-unknown-linux-gnu.tar.gz \
  --repo YamabikoLab/logcut
```

The same command can be used for the ARM64 archive or `SHA256SUMS`. Continue to verify `SHA256SUMS` as well because checksums and provenance serve different purposes.
