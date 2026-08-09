# logcut

Reduce AI token usage with concise failure summaries for WordPress development commands.

`logcut` is a Linux command-line tool for AI-assisted WordPress plugin and theme development. It keeps successful command output quiet and extracts the useful parts of failed output, so an AI coding assistant receives a focused result instead of a long stream of routine logs.

It is especially useful with PHPCS, PHPCBF, PHPUnit, PHPStan, `wp-scripts`, Stylelint, Composer, npm dependency installation, and other commands commonly used in WordPress development. Docker, Git, Jest, Vitest, and the other supported profiles remain available for the surrounding development workflow.

## Install from GitHub Release

Download `SHA256SUMS` and the archive matching your system from the `v1.0.0` GitHub Release:

```text
logcut-v1.0.0-x86_64-unknown-linux-gnu.tar.gz
logcut-v1.0.0-aarch64-unknown-linux-gnu.tar.gz
SHA256SUMS
```

