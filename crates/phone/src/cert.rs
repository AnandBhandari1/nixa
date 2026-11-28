use crate::files::{private_dir, write_private};
use crate::secret::{hex, random_bytes, sha256_bytes};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, NameConstraints, PKCS_ECDSA_P256_SHA256, SanType, SerialNumber,
};
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::OffsetDateTime;

pub const CA_DAYS: u64 = 3650;
pub const DOOR_DAYS: u64 = 397;
pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;
pub const RENEW_MS: u64 = 30 * DAY_MS;
pub const CA_NAME: &str = "Nixa phone root";
pub const DOOR_NAME: &str = "Nixa phone door";
pub const ORG: &str = "Nixa";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoorNames {
    pub ips: Vec<String>,
    pub dns: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DoorCert {
    pub key_pem: String,
    pub cert_pem: String,
    pub ca_pem: String,
    pub ca_der: Vec<u8>,
    pub until: u64,
}

#[derive(Serialize, Deserialize)]
struct CaMeta {
    until: u64,
}

#[derive(Serialize, Deserialize)]
struct DoorMeta {
    names: DoorNames,
    until: u64,
    ca: String,
}

struct Files {
    ca_key: PathBuf,
    ca_pem: PathBuf,
    ca_json: PathBuf,
    door_key: PathBuf,
    door_pem: PathBuf,
    door_json: PathBuf,
}

fn files(dir: &Path) -> Files {
    Files {
        ca_key: dir.join("ca.key"),
        ca_pem: dir.join("ca.pem"),
        ca_json: dir.join("ca.json"),
        door_key: dir.join("door.key"),
        door_pem: dir.join("door.pem"),
        door_json: dir.join("door.json"),
    }
}

fn sorted(names: &DoorNames) -> DoorNames {
    let mut ips = names.ips.clone();
    let mut dns = names.dns.clone();
    ips.sort();
    ips.dedup();
    dns.sort();
    dns.dedup();
    DoorNames { ips, dns }
}

fn at(ms: u64) -> Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp((ms / 1000) as i64).map_err(|e| anyhow!("bad time: {e}"))
}

fn serial() -> Result<SerialNumber> {
    let mut bytes = random_bytes(16)?;
    bytes[0] &= 0x7f;
    bytes[0] |= 0x01;
    Ok(SerialNumber::from_slice(&bytes))
}

fn name(common: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common);
    dn.push(DnType::OrganizationName, ORG);
    dn
}

fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    params.distinguished_name = name(CA_NAME);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([10, 0, 0, 0], 8)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([172, 16, 0, 0], 12)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([192, 168, 0, 0], 16)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([127, 0, 0, 0], 8)),
            GeneralSubtree::DnsName("local".into()),
            GeneralSubtree::DnsName("localhost".into()),
        ],
        excluded_subtrees: Vec::new(),
    });
    params
}

pub fn pem_der(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem.lines().filter(|line| !line.starts_with("-----")).collect::<Vec<_>>().join("");
    let der = STANDARD.decode(body.trim()).context("the certificate file is not valid PEM")?;
    if der.is_empty() {
        bail!("the certificate file is empty");
    }
    Ok(der)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn ensure_root(dir: &Path, now: u64) -> Result<bool> {
    let f = files(dir);
    let meta: Option<CaMeta> = read_json(&f.ca_json);
    let fresh = meta.is_some_and(|m| m.until.saturating_sub(now) > RENEW_MS);
    if fresh && f.ca_key.exists() && f.ca_pem.exists() {
        return Ok(false);
    }
    private_dir(dir)?;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("could not make the root key")?;
    let until = now + CA_DAYS * DAY_MS;
    let mut params = ca_params();
    params.not_before = at(now.saturating_sub(DAY_MS))?;
    params.not_after = at(until)?;
    params.serial_number = Some(serial()?);
    let cert = params.self_signed(&key).context("could not make the root certificate")?;
    write_private(&f.ca_key, key.serialize_pem().as_bytes())?;
    write_private(&f.ca_pem, cert.pem().as_bytes())?;
    write_private(&f.ca_json, serde_json::to_string(&CaMeta { until })?.as_bytes())?;
    let _ = fs::remove_file(&f.door_pem);
    let _ = fs::remove_file(&f.door_json);
    Ok(true)
}

pub fn door_due(dir: &Path, names: &DoorNames, now: u64) -> bool {
    let f = files(dir);
    let Ok(ca_pem) = fs::read_to_string(&f.ca_pem) else {
        return true;
    };
    let Ok(ca_der) = pem_der(&ca_pem) else {
        return true;
    };
    let Some(meta) = read_json::<DoorMeta>(&f.door_json) else {
        return true;
    };
    !(f.door_key.exists()
        && f.door_pem.exists()
        && sorted(&meta.names) == sorted(names)
        && meta.ca == hex(&sha256_bytes(&ca_der))
        && meta.until.saturating_sub(now) > RENEW_MS)
}

pub fn ensure_door(dir: &Path, names: &DoorNames, now: u64) -> Result<bool> {
    if names.ips.is_empty() && names.dns.is_empty() {
        bail!("the door certificate needs an address");
    }
    if !door_due(dir, names, now) {
        return Ok(false);
    }
    let f = files(dir);
    let ca_pem = fs::read_to_string(&f.ca_pem).context("the root certificate is missing")?;
    let ca_der = pem_der(&ca_pem)?;
    let ca_key = KeyPair::from_pem(&fs::read_to_string(&f.ca_key).context("the root key is missing")?).context("the root key is broken")?;
    let issuer = Issuer::new(ca_params(), ca_key);
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("could not make the door key")?;
    let mut params = CertificateParams::default();
    params.distinguished_name = name(DOOR_NAME);
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let mut sans = Vec::new();
    for ip in &names.ips {
        let ip: IpAddr = ip.parse().with_context(|| format!("bad address {ip}"))?;
        sans.push(SanType::IpAddress(ip));
    }
    for dns in &names.dns {
        sans.push(SanType::DnsName(dns.clone().try_into().with_context(|| format!("bad name {dns}"))?));
    }
    params.subject_alt_names = sans;
    let until = now + DOOR_DAYS * DAY_MS;
    params.not_before = at(now.saturating_sub(DAY_MS))?;
    params.not_after = at(until)?;
    params.serial_number = Some(serial()?);
    let cert = params.signed_by(&key, &issuer).context("could not sign the door certificate")?;
    write_private(&f.door_key, key.serialize_pem().as_bytes())?;
    write_private(&f.door_pem, cert.pem().as_bytes())?;
    let meta = DoorMeta { names: sorted(names), until, ca: hex(&sha256_bytes(&ca_der)) };
    write_private(&f.door_json, serde_json::to_string(&meta)?.as_bytes())?;
    Ok(true)
}

pub fn door_cert(dir: &Path, names: &DoorNames, now: u64) -> Result<DoorCert> {
    ensure_root(dir, now)?;
    ensure_door(dir, names, now)?;
    let f = files(dir);
    let ca_pem = fs::read_to_string(&f.ca_pem)?;
    let meta: DoorMeta = read_json(&f.door_json).ok_or_else(|| anyhow!("the door certificate notes are missing"))?;
    Ok(DoorCert {
        key_pem: fs::read_to_string(&f.door_key)?,
        cert_pem: fs::read_to_string(&f.door_pem)?,
        ca_der: pem_der(&ca_pem)?,
        ca_pem,
        until: meta.until,
    })
}

pub fn tls_config(cert: &DoorCert) -> Result<Arc<ServerConfig>> {
    let chain = vec![CertificateDer::from_pem_slice(cert.cert_pem.as_bytes()).context("the door certificate is broken")?];
    let key = PrivateKeyDer::from_pem_slice(cert.key_pem.as_bytes()).context("the door key is broken")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn names(ip: &str) -> DoorNames {
        DoorNames { ips: vec![ip.into()], dns: vec!["mac.local".into()] }
    }

    #[test]
    fn root_and_door_are_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let certs = dir.path().join("phone");
        let now = 1_800_000_000_000;
        assert!(ensure_root(&certs, now).unwrap());
        assert!(!ensure_root(&certs, now + DAY_MS).unwrap());
        assert!(ensure_door(&certs, &names("192.168.1.5"), now).unwrap());
        assert!(!ensure_door(&certs, &names("192.168.1.5"), now + DAY_MS).unwrap());
        let mode = fs::metadata(&certs).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        for file in ["ca.key", "door.key"] {
            let mode = fs::metadata(certs.join(file)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn door_is_renewed_for_new_names_and_near_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_800_000_000_000;
        let first = door_cert(dir.path(), &names("192.168.1.5"), now).unwrap();
        let moved = door_cert(dir.path(), &names("10.0.0.7"), now).unwrap();
        assert_ne!(first.cert_pem, moved.cert_pem);
        assert_eq!(first.ca_pem, moved.ca_pem);
        let late = now + (DOOR_DAYS * DAY_MS) - RENEW_MS + DAY_MS;
        let renewed = door_cert(dir.path(), &names("10.0.0.7"), late).unwrap();
        assert_ne!(renewed.cert_pem, moved.cert_pem);
        assert_eq!(renewed.ca_pem, moved.ca_pem);
        assert!(renewed.until > moved.until);
    }

    #[test]
    fn new_root_near_its_end_makes_a_new_door() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_800_000_000_000;
        let first = door_cert(dir.path(), &names("192.168.1.5"), now).unwrap();
        let late = now + CA_DAYS * DAY_MS - RENEW_MS + DAY_MS;
        let next = door_cert(dir.path(), &names("192.168.1.5"), late).unwrap();
        assert_ne!(first.ca_pem, next.ca_pem);
        assert_ne!(first.cert_pem, next.cert_pem);
    }

    #[test]
    fn tls_config_loads_the_door_files() {
        let dir = tempfile::tempdir().unwrap();
        let cert = door_cert(dir.path(), &names("127.0.0.1"), crate::now_ms()).unwrap();
        let config = tls_config(&cert).unwrap();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
        assert!(!cert.ca_der.is_empty());
    }

    #[test]
    fn door_needs_an_address() {
        let dir = tempfile::tempdir().unwrap();
        assert!(door_cert(dir.path(), &DoorNames::default(), crate::now_ms()).is_err());
    }
}
