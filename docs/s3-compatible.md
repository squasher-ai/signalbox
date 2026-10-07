# S3-compatible storage

Squasher Signalbox uses the AWS SDK default credential chain for native AWS S3. It accepts `s3://bucket/prefix` targets and an optional absolute HTTP(S) endpoint. An endpoint cannot contain credentials, a query, or a fragment. For a custom endpoint, set explicit `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`; the default profile and instance credential chain is deliberately disabled to prevent sending credentials to an unexpected host.

The endpoint path is forced to path-style addressing for MinIO and LocalStack. Each object is written with `If-None-Match: *`, so an existing object cannot be silently overwritten. Use a new prefix for each run; the writer does not claim or clear unrelated objects already under that prefix.

Minimum permissions for a run are:

- `s3:PutObject` on the selected prefix
- `s3:HeadBucket` (or equivalent bucket access check, depending on provider)

For local MinIO:

```sh
mc mb local/fixtures
AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin \
  squasher-signalbox generate \
  --output s3://fixtures/agent-runs/example \
  --endpoint http://127.0.0.1:9000
```

Use a new prefix for each retry. A conditional write failure is reported as `output_exists`.
