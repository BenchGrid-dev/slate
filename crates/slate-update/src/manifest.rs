//! The release manifest: what the newest SlateOS is, where its source is, and the
//! signature that says the release key wrote it.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ring::signature::KeyPair;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// The manifest format this updater reads.
pub const FORMAT: u32 = 1;
const MANIFEST_LIMIT: u64 = 1 << 20;
const SIGNATURE_LIMIT: u64 = 16 << 10;
/// The Slate source is a few megabytes; this bounds a download that never ends.
const SOURCE_LIMIT: u64 = 512 << 20;

/// `0.3.1`: numbers and dots. `0.3` and `0.3.0` are the same version.
#[derive(Clone, Debug)]
pub struct Version(Vec<u64>);

impl Version {
    fn part(&self, i: usize) -> u64 {
        self.0.get(i).copied().unwrap_or(0)
    }
}

impl FromStr for Version {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.is_empty() || parts.len() > 4 {
            bail!("\"{s}\" is not a version");
        }
        let mut numbers = Vec::new();
        for part in parts {
            if part.is_empty() || part.len() > 6 || !part.bytes().all(|b| b.is_ascii_digit()) {
                bail!("\"{s}\" is not a version");
            }
            numbers.push(part.parse()?);
        }
        Ok(Self(numbers))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: Vec<String> = self.0.iter().map(u64::to_string).collect();
        f.write_str(&text.join("."))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let n = self.0.len().max(other.0.len());
        (0..n)
            .map(|i| self.part(i).cmp(&other.part(i)))
            .find(|o| o.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Version {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    /// The day of the release, `2026-12-01`.
    #[serde(default)]
    pub released: String,
    /// A sentence or two for the person, from the changelog.
    #[serde(default)]
    pub summary: String,
    /// Where the release notes are.
    #[serde(default)]
    pub notes: String,
    pub source: Source,
    /// The oldest version that can go to this one directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The manifest of the release an older system takes first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

/// The Slate source of the release: a gzipped tar with one directory at its top.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Source {
    /// Absolute, or relative to the manifest's own address.
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest: Self =
            serde_json::from_slice(bytes).context("the release manifest is not valid")?;
        if manifest.format != FORMAT {
            bail!(
                "the release manifest has format {}, and this version of SlateOS reads format {FORMAT}",
                manifest.format
            );
        }
        manifest.version()?;
        if let Some(from) = &manifest.from {
            Version::from_str(from)?;
        }
        if manifest.source.sha256.len() != 64
            || !manifest
                .source
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            bail!("the release manifest names no SHA-256 for the source");
        }
        Ok(manifest)
    }

    pub fn version(&self) -> Result<Version> {
        Version::from_str(&self.version)
    }
}

/// A manifest and the address it came from (its source may be named relative to it).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Offer {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub manifest_url: String,
}

impl Offer {
    pub fn source_url(&self) -> String {
        resolve(&self.manifest_url, &self.manifest.source.url)
    }
}

/// `reference` as an address: itself when it has a scheme, else next to `base`.
pub fn resolve(base: &str, reference: &str) -> String {
    if reference.contains("://") {
        return reference.to_string();
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{}", &base[..i], reference.trim_start_matches("./")),
        None => reference.to_string(),
    }
}

/// Accepts the manifest when one of the signatures is one of the keys' over exactly
/// these bytes. `signatures` holds one base64 Ed25519 signature a line (two while a
/// key is being replaced).
pub fn verify(manifest: &[u8], signatures: &str, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        bail!("this system trusts no release key (services.slate.update.publicKeys)");
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let keys: Vec<Vec<u8>> = keys
        .iter()
        .filter_map(|k| b64.decode(k.trim()).ok())
        .filter(|k| k.len() == 32)
        .collect();
    for line in signatures
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let Ok(signature) = b64.decode(line) else {
            continue;
        };
        for key in &keys {
            let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key);
            if key.verify(manifest, &signature).is_ok() {
                return Ok(());
            }
        }
    }
    bail!("the release manifest is not signed with a key this system trusts")
}

/// A new release key: the secret half (PKCS#8, base64) and the public half (base64).
pub fn generate_key() -> Result<(String, String)> {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| anyhow!("could not generate a key"))?;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|e| anyhow!("generated key rejected: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD;
    Ok((
        b64.encode(pkcs8.as_ref()),
        b64.encode(pair.public_key().as_ref()),
    ))
}

fn key_pair(secret: &str) -> Result<ring::signature::Ed25519KeyPair> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let pkcs8 = b64
        .decode(secret.trim())
        .context("the key file is not base64")?;
    ring::signature::Ed25519KeyPair::from_pkcs8(&pkcs8)
        .map_err(|e| anyhow!("the key file is not an Ed25519 key: {e}"))
}

/// The signature line of `secret` over `message`.
pub fn sign(secret: &str, message: &[u8]) -> Result<String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    Ok(b64.encode(key_pair(secret)?.sign(message).as_ref()))
}

/// The public half of a secret key, as `publicKeys` wants it.
pub fn public_key(secret: &str) -> Result<String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    Ok(b64.encode(key_pair(secret)?.public_key().as_ref()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Gets files over HTTPS, or from `file://` (a test, a USB stick).
pub struct Fetcher {
    agent: ureq::Agent,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_global(Some(Duration::from_secs(900)))
            .user_agent(concat!("slateos-update/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }

    fn open(&self, url: &str) -> Result<Box<dyn Read>> {
        if let Some(path) = url.strip_prefix("file://") {
            let file = std::fs::File::open(path).with_context(|| format!("opening {path}"))?;
            return Ok(Box::new(file));
        }
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("{url} is not an address this can fetch");
        }
        let response = self.agent.get(url).call().map_err(|e| match e {
            ureq::Error::StatusCode(code) => anyhow!("{url} answered {code}"),
            other => anyhow!("{url} could not be reached: {other}"),
        })?;
        Ok(Box::new(response.into_body().into_reader()))
    }

    /// A small file, whole.
    pub fn bytes(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.open(url)?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading {url}"))?;
        if bytes.len() as u64 > limit {
            bail!("{url} is larger than the {limit} bytes it may be");
        }
        Ok(bytes)
    }

    /// A download into `dest`; returns its SHA-256 and size. `progress` hears the
    /// bytes so far.
    pub fn to_file(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(String, u64)> {
        let mut reader = self.open(url)?;
        let mut file =
            std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
        let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = vec![0u8; 64 << 10];
        let mut total = 0u64;
        loop {
            let n = reader
                .read(&mut buffer)
                .with_context(|| format!("downloading {url}"))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > SOURCE_LIMIT {
                bail!("{url} is larger than a SlateOS source can be");
            }
            digest.update(&buffer[..n]);
            file.write_all(&buffer[..n])?;
            progress(total);
        }
        file.sync_all()?;
        Ok((hex(digest.finish().as_ref()), total))
    }

    /// The manifest at `url`, after its signature was checked.
    pub fn manifest(&self, url: &str, keys: &[String]) -> Result<Manifest> {
        let bytes = self.bytes(url, MANIFEST_LIMIT)?;
        let signatures = self.bytes(&format!("{url}.sig"), SIGNATURE_LIMIT)?;
        verify(&bytes, &String::from_utf8_lossy(&signatures), keys)?;
        Manifest::parse(&bytes)
    }

    /// The release a system at `current` takes next: the newest one, or the one the
    /// newest names as the way there for systems older than it can update. None when
    /// `current` is the newest (or newer: a version is never taken back this way).
    pub fn find(&self, url: &str, keys: &[String], current: &Version) -> Result<Option<Offer>> {
        let mut url = url.to_string();
        for hop in 0..6 {
            let manifest = self.manifest(&url, keys)?;
            let version = manifest.version()?;
            if version <= *current {
                if hop == 0 {
                    return Ok(None);
                }
                bail!("the way to the newest SlateOS leads through {version}, which is not newer than this system's {current}");
            }
            let from = manifest
                .from
                .as_deref()
                .map(Version::from_str)
                .transpose()?;
            if from.is_some_and(|from| *current < from) {
                match &manifest.step {
                    Some(step) => {
                        url = resolve(&url, step);
                        continue;
                    }
                    None => bail!(
                        "SlateOS {version} cannot be installed over {current}, and its release names no version in between"
                    ),
                }
            }
            return Ok(Some(Offer {
                manifest,
                manifest_url: url,
            }));
        }
        bail!("the releases name each other as the way to the newest one")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(v("0.3.1") > v("0.3.0"));
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0") > v("0.99.99"));
        assert_eq!(v("0.3"), v("0.3.0"));
        assert!(v("0.3.0.1") > v("0.3"));
        assert_eq!(v("0.3.1").to_string(), "0.3.1");
        for bad in [
            "",
            "0.3.x",
            "v0.3",
            "0..3",
            "1.2.3.4.5",
            "0.3-rc1",
            "0.1234567",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
    }

    #[test]
    fn addresses_next_to_the_manifest() {
        let base = "https://example.org/releases/latest/slateos-update.json";
        assert_eq!(
            resolve(base, "slateos-0.3.1-source.tar.gz"),
            "https://example.org/releases/latest/slateos-0.3.1-source.tar.gz"
        );
        assert_eq!(
            resolve(base, "./x.tar.gz"),
            "https://example.org/releases/latest/x.tar.gz"
        );
        assert_eq!(
            resolve(base, "https://other.example/x"),
            "https://other.example/x"
        );
        assert_eq!(
            resolve("file:///m/u.json", "s.tar.gz"),
            "file:///m/s.tar.gz"
        );
    }

    fn manifest(version: &str) -> String {
        format!(
            r#"{{"format":1,"version":"{version}","released":"2026-12-01","summary":"s","notes":"n",
               "source":{{"url":"src.tar.gz","sha256":"{}","size":3}}, "later":"ignored"}}"#,
            "ab".repeat(32)
        )
    }

    #[test]
    fn a_signature_must_be_a_trusted_keys_over_these_bytes() {
        let (secret, public) = generate_key().unwrap();
        let (other_secret, other_public) = generate_key().unwrap();
        assert_eq!(public_key(&secret).unwrap(), public);
        let text = manifest("0.3.1");
        let signature = sign(&secret, text.as_bytes()).unwrap();

        verify(text.as_bytes(), &signature, std::slice::from_ref(&public)).unwrap();
        // Two keys trusted, two signatures given, comments and blank lines between.
        let both = format!(
            "# old key\n{}\n\n{signature}\n",
            sign(&other_secret, text.as_bytes()).unwrap()
        );
        verify(text.as_bytes(), &both, std::slice::from_ref(&public)).unwrap();
        verify(
            text.as_bytes(),
            &signature,
            &[other_public.clone(), public.clone()],
        )
        .unwrap();

        // Another key, changed bytes, nothing trusted, rubbish.
        assert!(verify(
            text.as_bytes(),
            &signature,
            std::slice::from_ref(&other_public)
        )
        .is_err());
        let changed = text.replace("0.3.1", "0.3.2");
        assert!(verify(
            changed.as_bytes(),
            &signature,
            std::slice::from_ref(&public)
        )
        .is_err());
        assert!(verify(text.as_bytes(), &signature, &[]).is_err());
        assert!(verify(
            text.as_bytes(),
            "not base64 !!",
            std::slice::from_ref(&public)
        )
        .is_err());
        assert!(verify(text.as_bytes(), "", std::slice::from_ref(&public)).is_err());
    }

    #[test]
    fn a_manifest_is_checked_when_read() {
        let good = Manifest::parse(manifest("0.3.1").as_bytes()).unwrap();
        assert_eq!(good.version, "0.3.1");
        assert_eq!(good.source.size, 3);
        assert!(Manifest::parse(
            manifest("0.3.1")
                .replace("\"format\":1", "\"format\":2")
                .as_bytes()
        )
        .is_err());
        assert!(Manifest::parse(manifest("three").as_bytes()).is_err());
        assert!(Manifest::parse(
            manifest("0.3.1")
                .replace(&"ab".repeat(32), "abc")
                .as_bytes()
        )
        .is_err());
        assert!(Manifest::parse(b"{}").is_err());
    }

    /// A release directory on disk: manifests signed with `secret`.
    fn publish(dir: &Path, name: &str, body: &str, secret: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::write(
            dir.join(format!("{name}.sig")),
            sign(secret, body.as_bytes()).unwrap(),
        )
        .unwrap();
        format!("file://{}", path.display())
    }

    #[test]
    fn the_next_release_is_the_newest_or_the_step_towards_it() {
        let dir = tempfile::tempdir().unwrap();
        let (secret, public) = generate_key().unwrap();
        let keys = [public];
        let fetcher = Fetcher::new();

        let url = publish(dir.path(), "latest.json", &manifest("0.3.1"), &secret);
        let offer = fetcher.find(&url, &keys, &v("0.3.0")).unwrap().unwrap();
        assert_eq!(offer.manifest.version, "0.3.1");
        assert!(offer.source_url().ends_with("/src.tar.gz"));
        assert!(fetcher.find(&url, &keys, &v("0.3.1")).unwrap().is_none());
        // A system ahead of the release is not taken back.
        assert!(fetcher.find(&url, &keys, &v("0.4.0")).unwrap().is_none());

        // 0.5.0 wants at least 0.4.0 and names 0.4.2 as the way there.
        publish(dir.path(), "step.json", &manifest("0.4.2"), &secret);
        let newest = manifest("0.5.0").replace(
            "\"later\"",
            "\"from\":\"0.4.0\",\"step\":\"step.json\",\"later\"",
        );
        let url = publish(dir.path(), "newest.json", &newest, &secret);
        assert_eq!(
            fetcher
                .find(&url, &keys, &v("0.3.0"))
                .unwrap()
                .unwrap()
                .manifest
                .version,
            "0.4.2"
        );
        assert_eq!(
            fetcher
                .find(&url, &keys, &v("0.4.0"))
                .unwrap()
                .unwrap()
                .manifest
                .version,
            "0.5.0"
        );

        // No step named: say so instead of installing what cannot work.
        let stuck = manifest("0.5.0").replace("\"later\"", "\"from\":\"0.4.0\",\"later\"");
        let url = publish(dir.path(), "stuck.json", &stuck, &secret);
        assert!(fetcher.find(&url, &keys, &v("0.3.0")).is_err());

        // A manifest whose signature is another key's is refused.
        let (stranger, _) = generate_key().unwrap();
        let url = publish(dir.path(), "forged.json", &manifest("9.9.9"), &stranger);
        assert!(fetcher.find(&url, &keys, &v("0.3.0")).is_err());
        // And one without a signature file.
        std::fs::write(dir.path().join("bare.json"), manifest("9.9.9")).unwrap();
        let url = format!("file://{}", dir.path().join("bare.json").display());
        assert!(fetcher.find(&url, &keys, &v("0.3.0")).is_err());
    }

    #[test]
    fn a_download_reports_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"slate").unwrap();
        let mut seen = 0;
        let (hash, size) = Fetcher::new()
            .to_file(
                &format!("file://{}", dir.path().join("a").display()),
                &dir.path().join("b"),
                &mut |n| seen = n,
            )
            .unwrap();
        assert_eq!(size, 5);
        assert_eq!(seen, 5);
        assert_eq!(hash, sha256_hex(b"slate"));
        assert_eq!(std::fs::read(dir.path().join("b")).unwrap(), b"slate");
    }
}
