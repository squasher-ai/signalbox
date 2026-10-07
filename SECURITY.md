# Security policy

Squasher Signalbox generates synthetic data. Never use it with real prompts, customer telemetry, credentials, or private identifiers.

## Supported versions

Security fixes are applied to the latest release on the default branch. Older releases may not receive patches.

## Report a vulnerability

Please use [GitHub's private security advisory form](https://github.com/squasher-ai/squasher-signalbox/security/advisories/new). Do not open a public issue for a vulnerability or include secrets in a report. Include the affected version, a minimal reproduction, and the impact. We will acknowledge a report through GitHub and coordinate a fix and disclosure timeline there.

The CLI accepts an S3-compatible endpoint but rejects credentials in the URL and does not print environment credentials. Review endpoint and bucket permissions before running against shared storage.
