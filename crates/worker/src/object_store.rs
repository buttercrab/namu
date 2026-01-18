use std::net::IpAddr;

use namu_proto::ValueRef;
use reqsign_aws_v4::{Credential, EMPTY_STRING_SHA256, RequestSigner, StaticCredentialProvider};
use reqsign_core::{Context as SignContext, Signer};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Url};

#[derive(Clone)]
pub struct ObjectStore {
    client: Client,
    endpoint: Url,
    bucket: String,
    force_path_style: bool,
    signer: Option<Signer<Credential>>,
}

impl ObjectStore {
    pub async fn from_env() -> anyhow::Result<Option<Self>> {
        let endpoint = match std::env::var("NAMU_OBJECT_STORE_ENDPOINT") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => return Ok(None),
        };

        let endpoint = Url::parse(&endpoint)?;
        let bucket =
            std::env::var("NAMU_OBJECT_STORE_BUCKET").unwrap_or_else(|_| "namu".to_string());
        let access_key = std::env::var("NAMU_OBJECT_STORE_ACCESS_KEY")
            .unwrap_or_else(|_| "minioadmin".to_string());
        let secret_key = std::env::var("NAMU_OBJECT_STORE_SECRET_KEY")
            .unwrap_or_else(|_| "minioadmin".to_string());
        let region =
            std::env::var("NAMU_OBJECT_STORE_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let force_path_style = std::env::var("NAMU_OBJECT_STORE_FORCE_PATH_STYLE")
            .ok()
            .and_then(|value| parse_bool(&value))
            .unwrap_or(true);

        let signer = if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            None
        } else {
            let provider = StaticCredentialProvider::new(&access_key, &secret_key);
            Some(Signer::new(
                SignContext::new(),
                provider,
                RequestSigner::new("s3", &region),
            ))
        };

        Ok(Some(Self {
            client: Client::new(),
            endpoint,
            bucket,
            force_path_style,
            signer,
        }))
    }

    pub async fn get_json(&self, value_ref: &ValueRef) -> anyhow::Result<Vec<u8>> {
        let (bucket, key) = parse_ref(&self.bucket, &value_ref.ref_uri)?;
        if bucket != self.bucket {
            return Err(anyhow::anyhow!(
                "unexpected bucket {bucket} (expected {})",
                self.bucket
            ));
        }

        let url = self.object_url(&key)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-content-sha256",
            HeaderValue::from_static(EMPTY_STRING_SHA256),
        );
        let headers = self.signed_headers("GET", &url, headers).await?;

        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    async fn signed_headers(
        &self,
        method: &str,
        url: &Url,
        headers: HeaderMap,
    ) -> anyhow::Result<HeaderMap> {
        let Some(signer) = &self.signer else {
            return Ok(headers);
        };
        let req = http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .body(())?;
        let (mut parts, _) = req.into_parts();
        parts.headers = headers;
        signer.sign(&mut parts, None).await?;
        Ok(parts.headers)
    }

    fn object_url(&self, key: &str) -> anyhow::Result<Url> {
        let mut url = self.endpoint.clone();
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("missing endpoint host"))?;
        let use_virtual_host = !self.force_path_style && !is_path_only_host(host);

        if use_virtual_host {
            let virtual_host = format!("{}.{}", self.bucket, host);
            url.set_host(Some(&virtual_host))?;
        }

        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("endpoint cannot be a base"))?;
            segments.pop_if_empty();
            if !use_virtual_host {
                segments.push(&self.bucket);
            }
            for segment in key.split('/') {
                if !segment.is_empty() {
                    segments.push(segment);
                }
            }
        }

        Ok(url)
    }
}

fn is_path_only_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok()
}

fn parse_ref(default_bucket: &str, ref_uri: &str) -> anyhow::Result<(String, String)> {
    if let Some(rest) = ref_uri.strip_prefix("s3://") {
        let mut parts = rest.splitn(2, '/');
        let bucket = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("invalid s3 ref"))?;
        let key = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("invalid s3 ref"))?;
        return Ok((bucket.to_string(), key.to_string()));
    }

    Ok((
        default_bucket.to_string(),
        ref_uri.trim_start_matches('/').to_string(),
    ))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
