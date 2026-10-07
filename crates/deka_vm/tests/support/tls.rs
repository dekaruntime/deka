use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
pub struct Identity {
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
    pub ca: Vec<u8>,
}
pub fn identity() -> Identity {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(vec![]).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca.distinguished_name
        .push(rcgen::DnType::CommonName, "ephemeral Deka test CA");
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let key = KeyPair::generate().unwrap();
    let mut leaf = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let cert = leaf
        .signed_by(&key, &Issuer::from_params(&ca, &ca_key))
        .unwrap();
    Identity {
        cert: cert.pem().into_bytes(),
        key: key.serialize_pem().into_bytes(),
        ca: ca_cert.pem().into_bytes(),
    }
}
pub fn program(guide: &str, i: &Identity) -> String {
    format!(
        "{guide}\nasync fn main(){{\nconst cert=unwrap(from_hex(\"{}\")) or{{return;}};\nconst key=unwrap(from_hex(\"{}\")) or{{return;}};\nconst ca=unwrap(from_hex(\"{}\")) or{{return;}};\nconsole.log(await roundTrip(cert,key,ca));\n}}",
        hex(&i.cert),
        hex(&i.key),
        hex(&i.ca)
    )
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}
