use std::{
    env,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use aws_sdk_s3::config::{
    Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncWriteExt;

use crate::error::Error;

pub enum Sink {
    Local(PathBuf),
    S3 { client: aws_sdk_s3::Client, bucket: String, prefix: String },
}

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Sink {
    pub fn validate_output(output: &str) -> Result<(), Error> {
        if let Some(target) = output.strip_prefix("s3://") {
            parse_s3_target(target).map(|_| ())
        } else if output.is_empty() || output.contains("://") {
            Err(Error::new("invalid_output", "Use a local directory or s3://bucket/prefix."))
        } else {
            Ok(())
        }
    }

    pub async fn create(output: &str, endpoint: Option<&str>, region: &str) -> Result<Self, Error> {
        if let Some(target) = output.strip_prefix("s3://") {
            let (bucket, prefix) = parse_s3_target(target)?;
            let config = if let Some(endpoint) = endpoint {
                // A custom endpoint may be attacker-controlled. Do not let the AWS SDK
                // discover and forward instance/profile credentials to it implicitly.
                let credentials = endpoint_credentials_from_env()?;
                aws_sdk_s3::config::Builder::new()
                    .region(Region::new(region.to_owned()))
                    .credentials_provider(credentials)
                    .endpoint_url(endpoint)
                    .force_path_style(true)
                    .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
                    .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            } else {
                let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(Region::new(region.to_owned()))
                    .load()
                    .await;
                aws_sdk_s3::config::Builder::from(&shared)
            };
            return Ok(Self::S3 {
                client: aws_sdk_s3::Client::from_conf(config.build()),
                bucket,
                prefix,
            });
        }
        Self::validate_output(output)?;
        if endpoint.is_some() {
            return Err(Error::new(
                "invalid_output",
                "An S3 endpoint requires an S3 output target.",
            ));
        }
        let path = PathBuf::from(output);
        tokio::fs::create_dir(&path).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new("output_exists", "The output directory must be new.")
            } else {
                Error::new(
                    "output_create_failed",
                    "Cannot create the output directory. Check its parent and permissions.",
                )
            }
        })?;
        Ok(Self::Local(path))
    }

    pub async fn write(&self, name: &str, data: Vec<u8>, content_type: &str) -> Result<(), Error> {
        validate_name(name)?;
        match self {
            Self::Local(directory) => {
                write_local_atomically(directory, name, data).await?;
                Ok(())
            }
            Self::S3 { client, bucket, prefix } => {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(format!("{prefix}/{name}"))
                    .if_none_match("*")
                    .content_type(content_type)
                    .body(ByteStream::from(data))
                    .send()
                    .await
                    .map_err(|error| {
                        let code = error
                            .as_service_error()
                            .and_then(|service| service.meta().code());
                        if matches!(code, Some("PreconditionFailed" | "ConditionalRequestConflict")) {
                            Error::new("output_exists", "An S3 output object already exists.")
                        } else {
                            Error::new(
                                "output_write_failed",
                                "Cannot write an S3 object. Check credentials, endpoint, and access.",
                            )
                        }
                    })?;
                Ok(())
            }
        }
    }
}

/// Write a local object without ever exposing a partially written destination.
///
/// The temporary file is created in the destination directory, written and
/// synced before it is linked into place. A hard link gives us the
/// create-only/no-clobber property that `rename` does not provide on all
/// platforms. The temporary link is removed after the destination is visible.
async fn write_local_atomically(directory: &Path, name: &str, data: Vec<u8>) -> Result<(), Error> {
    let (temporary_path, mut file) = create_temporary_file(directory, name).await?;
    let destination = directory.join(name);

    let result = async {
        file.write_all(&data)
            .await
            .map_err(|_| Error::new("output_write_failed", "Cannot write an output file."))?;
        file.flush()
            .await
            .map_err(|_| Error::new("output_write_failed", "Cannot flush an output file."))?;
        file.sync_all()
            .await
            .map_err(|_| Error::new("output_write_failed", "Cannot sync an output file."))?;
        drop(file);

        tokio::fs::hard_link(&temporary_path, &destination).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new("output_exists", "An output file already exists.")
            } else {
                Error::new("output_write_failed", "Cannot create an output file.")
            }
        })
    }
    .await;

    // A failed write must not leave a misleading partial artifact. Cleanup is
    // best effort because the committed destination is already durable when
    // the link operation succeeds.
    let _ = tokio::fs::remove_file(&temporary_path).await;
    result
}

async fn create_temporary_file(
    directory: &Path,
    name: &str,
) -> Result<(PathBuf, tokio::fs::File), Error> {
    for _ in 0..32 {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".{name}.partial-{}-{suffix}", std::process::id()));
        match tokio::fs::OpenOptions::new().write(true).create_new(true).open(&path).await {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(Error::new("output_write_failed", "Cannot create an output file."));
            }
        }
    }
    Err(Error::new("output_write_failed", "Cannot allocate a temporary output file."))
}

fn endpoint_credentials_from_env() -> Result<Credentials, Error> {
    static_endpoint_credentials(
        env::var("AWS_ACCESS_KEY_ID").ok().as_deref(),
        env::var("AWS_SECRET_ACCESS_KEY").ok().as_deref(),
        env::var("AWS_SESSION_TOKEN").ok().as_deref(),
    )
}

fn static_endpoint_credentials(
    access_key: Option<&str>,
    secret_key: Option<&str>,
    session_token: Option<&str>,
) -> Result<Credentials, Error> {
    let Some(access_key) = access_key.filter(|value| !value.is_empty()) else {
        return Err(Error::new(
            "missing_endpoint_credentials",
            "Custom S3 endpoints require AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.",
        ));
    };
    let Some(secret_key) = secret_key.filter(|value| !value.is_empty()) else {
        return Err(Error::new(
            "missing_endpoint_credentials",
            "Custom S3 endpoints require AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.",
        ));
    };
    Ok(Credentials::new(
        access_key,
        secret_key,
        session_token.filter(|value| !value.is_empty()).map(str::to_owned),
        None,
        "signalbox-static-endpoint",
    ))
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(Error::new("invalid_output_name", "An output name must be one file name."));
    }
    Ok(())
}

fn parse_s3_target(target: &str) -> Result<(String, String), Error> {
    let invalid = || Error::new("invalid_output", "S3 output requires s3://bucket/prefix.");
    let (bucket, prefix) = target.split_once('/').ok_or_else(invalid)?;
    let prefix = prefix.trim_end_matches('/');
    if !(3..=63).contains(&bucket.len())
        || !bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte))
        || !bucket.as_bytes()[0].is_ascii_alphanumeric()
        || !bucket.as_bytes()[bucket.len() - 1].is_ascii_alphanumeric()
        || bucket.contains("..")
        || prefix.is_empty()
        || prefix.contains(['?', '#', '\\'])
        || prefix.chars().any(char::is_control)
        || prefix.split('/').any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(invalid());
    }
    Ok((bucket.to_owned(), prefix.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_output_preserves_existing_files_and_stays_inside_target() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("output");
        let sink = Sink::create(path.to_str().unwrap(), None, "us-east-1").await.unwrap();
        sink.write("part.json", b"first".to_vec(), "application/json").await.unwrap();
        assert!(sink.write("part.json", b"second".to_vec(), "application/json").await.is_err());
        assert!(sink.write("../escape.json", vec![], "application/json").await.is_err());
        assert!(Sink::create(path.to_str().unwrap(), None, "us-east-1").await.is_err());
        assert_eq!(std::fs::read(path.join("part.json")).unwrap(), b"first");
        assert!(!temporary.path().join("escape.json").exists());
    }

    #[test]
    fn custom_endpoint_requires_explicit_static_credentials_before_network() {
        let error = static_endpoint_credentials(None, None, None).unwrap_err();
        assert_eq!(error.code, "missing_endpoint_credentials");
        assert!(error.message.contains("AWS_ACCESS_KEY_ID"));
        assert!(static_endpoint_credentials(Some("key"), None, None).is_err());
        assert!(static_endpoint_credentials(Some("key"), Some("secret"), None).is_ok());
    }

    #[test]
    fn s3_target_requires_a_bucket_and_safe_nonempty_prefix() {
        assert_eq!(
            parse_s3_target("my-bucket/fixtures/run-1/").unwrap(),
            ("my-bucket".to_owned(), "fixtures/run-1".to_owned())
        );
        for target in [
            "my-bucket",
            "my-bucket/",
            "my-bucket//",
            "my-bucket/../run",
            "my-bucket/run?token=hidden",
            "user@my-bucket/run",
        ] {
            assert!(parse_s3_target(target).is_err(), "accepted invalid S3 target");
        }
    }
}
