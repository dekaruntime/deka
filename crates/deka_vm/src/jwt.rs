//! HS256 compact JWT engine. Public DekaScript calls supply a checked JSON schema.
//! Shares the native crypto engine and accepts an explicit clock for verification.
use crate::json::JWT_MAX_INPUT as MAX_INPUT;
use crate::{Result, bytes, crypto};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Number, Value, value::RawValue};
use std::fmt;
const MAX_TOKEN: usize = MAX_INPUT + 44;

use crate::jwt_contract::{
    Claim, Kind, OPTIONS_ENCODING_ERROR, OptionField, PAYLOAD_ENCODING_ERROR, SIGN_HOST,
    VERIFY_HOST,
};
/// Retains source property order and nested JSON spelling produced by the
/// shared typed writer. Existing entries are replaced in place, like the old
/// boundary's assignment; new claims append in its documented order.
#[derive(Debug)]
struct Object {
    fields: Vec<(String, Box<RawValue>)>,
    index: std::collections::BTreeMap<String, usize>,
}
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Object;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JWT object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Object, A::Error> {
                let mut out = Object {
                    fields: Vec::new(),
                    index: Default::default(),
                };
                while let Some((name, value)) = access.next_entry::<String, Box<RawValue>>()? {
                    out.set(name, value);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}
impl Serialize for Object {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for (key, value) in &self.fields {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
impl Object {
    fn set(&mut self, name: String, value: Box<RawValue>) {
        if let Some(index) = self.index.get(&name) {
            self.fields[*index].1 = value;
        } else {
            self.index.insert(name.clone(), self.fields.len());
            self.fields.push((name, value));
        }
    }
    fn get(&self, name: &str) -> Result<Option<Value>> {
        self.index
            .get(name)
            .map(|index| {
                serde_json::from_str(self.fields[*index].1.get()).map_err(|e| e.to_string())
            })
            .transpose()
    }
    fn put(&mut self, name: &str, value: Value) -> Result<()> {
        self.set(
            name.into(),
            RawValue::from_string(serde_json::to_string(&value).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        );
        Ok(())
    }
    fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_INPUT {
            return Err("input too large".into());
        }
        // Shared serde JSON validation rejects non-finite numbers and excessive nesting
        // before retaining raw fields, including unknown custom properties.
        let _: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
    fn stringify(&self) -> Result<String> {
        struct Limited(Vec<u8>);
        impl std::io::Write for Limited {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if data.len() > MAX_INPUT - self.0.len() {
                    return Err(std::io::Error::other("input too large"));
                }
                self.0.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Limited(Vec::new());
        serde_json::to_writer(&mut writer, self).map_err(|e| e.to_string())?;
        String::from_utf8(writer.0).map_err(|e| e.to_string())
    }
    fn number(&self, name: &str) -> Result<Option<f64>> {
        match self.get(name)? {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => n
                .as_f64()
                .filter(|n| n.is_finite())
                .map(Some)
                .ok_or_else(|| "invalid JWT numeric claim".into()),
            _ => Err("invalid JWT numeric claim".into()),
        }
    }
    fn text(&self, name: &str) -> Result<Option<String>> {
        match self.get(name)? {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            _ => Err("invalid JWT string claim".into()),
        }
    }
    fn validate_claims(&self) -> Result<()> {
        for field in Claim::ALL {
            match field.kind() {
                Kind::Number => {
                    self.number(field.name())?;
                }
                Kind::String => {
                    self.text(field.name())?;
                }
            }
        }
        Ok(())
    }
}
fn options(text: &str) -> Result<Object> {
    let object = Object::parse(text).map_err(|_| OPTIONS_ENCODING_ERROR.to_owned())?;
    for field in OptionField::ALL {
        match field.kind() {
            Kind::Number => {
                object.number(field.name())?;
            }
            Kind::String => {
                object.text(field.name())?;
            }
        }
    }
    Ok(object)
}
fn numeric(value: f64) -> Result<Value> {
    if !value.is_finite() {
        return Err("invalid JWT numeric claim".into());
    }
    if value.fract() == 0. && value >= i64::MIN as f64 && value < i64::MAX as f64 {
        Ok(Value::Number(Number::from(value as i64)))
    } else {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| "invalid JWT numeric claim".into())
    }
}
fn b64url(data: &[u8]) -> String {
    bytes::to_base64(data)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}
fn valid_b64url(value: &str) -> bool {
    !value.is_empty()
        && value.len() % 4 != 1
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn decode(value: &str) -> Result<Vec<u8>> {
    let mut padded = value.replace('-', "+").replace('_', "/");
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    bytes::from_base64(&padded).ok_or_else(|| "invalid base64url".into())
}
/// Clock is explicit so timestamp/expiry decisions are reproducible in tests.
pub fn sign(payload: &str, secret: &[u8], option_json: &str, now: f64) -> Result<String> {
    if !now.is_finite() {
        return Err("invalid JWT clock".into());
    }
    if secret.len() > MAX_INPUT {
        return Err("input too large".into());
    }
    let options = options(option_json)?;
    let alg = options
        .text("alg")?
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "HS256".into())
        .to_uppercase();
    if alg != "HS256" {
        return Err("unsupported jwt alg".into());
    }
    let mut claims = Object::parse(payload).map_err(|_| PAYLOAD_ENCODING_ERROR.to_owned())?;
    claims
        .validate_claims()
        .map_err(|_| PAYLOAD_ENCODING_ERROR.to_owned())?;
    let iat = claims.number("iat")?.unwrap_or(now);
    let relative_exp = match options.number("exp_in")? {
        Some(n) => Some(now + n),
        None => claims.number("exp")?,
    };
    let exp = options.number("exp")?.or(relative_exp);
    for name in ["iss", "aud", "sub"] {
        if let Some(value) = options.text(name)? {
            claims.put(name, Value::String(value))?;
        }
    }
    claims.put("iat", numeric(iat)?)?;
    if let Some(exp) = exp {
        claims.put("exp", numeric(exp)?)?;
    }
    let json = claims.stringify()?;
    let input = format!(
        "{}.{}",
        b64url(br#"{"alg":"HS256","typ":"JWT"}"#),
        b64url(json.as_bytes())
    );
    let signature = crypto::hmac("sha256", secret, input.as_bytes())?;
    Ok(format!("{input}.{}", b64url(&signature)))
}
/// Returns authenticated JSON for the shared statically typed decoder. Custom
/// claims remain in this JSON; the public API exposes its declared claim fields.
pub fn verify(token: &str, secret: &[u8], option_json: &str, now: f64) -> Result<String> {
    if !now.is_finite() {
        return Err("invalid JWT clock".into());
    }
    if token.len() > MAX_TOKEN || secret.len() > MAX_INPUT {
        return Err("input too large".into());
    }
    let options = options(option_json)?;
    let mut pieces = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (pieces.next(), pieces.next(), pieces.next(), pieces.next())
    else {
        return Err("invalid jwt format".into());
    };
    if ![header, payload, signature]
        .iter()
        .all(|value| valid_b64url(value))
    {
        return Err("invalid jwt format".into());
    }
    let header = String::from_utf8(decode(header)?).map_err(|_| "invalid jwt header".to_owned())?;
    let header = Object::parse(&header).map_err(|_| "invalid jwt header".to_owned())?;
    let alg = header
        .text("alg")
        .map_err(|_| "invalid jwt header".to_owned())?
        .unwrap_or_default();
    if alg != "HS256" {
        return Err("unsupported jwt alg".into());
    }
    let split = token.rfind('.').ok_or("invalid jwt format")?;
    let expected = crypto::hmac("sha256", secret, &token.as_bytes()[..split])?;
    let actual = decode(signature).map_err(|_| "invalid jwt signature".to_owned())?;
    if !crypto::secure_compare(&expected, &actual)? {
        return Err("invalid jwt signature".into());
    }
    let json = String::from_utf8(decode(payload)?).map_err(|_| "invalid jwt payload".to_owned())?;
    let mut claims = Object::parse(&json).map_err(|_| "invalid jwt payload".to_owned())?;
    claims
        .validate_claims()
        .map_err(|_| "invalid jwt payload".to_owned())?;
    let leeway = options.number("leeway")?.unwrap_or(0.);
    if claims.number("iat")?.is_some_and(|n| n - leeway > now) {
        return Err("jwt issued in future".into());
    }
    if claims.number("exp")?.is_some_and(|n| n + leeway < now) {
        return Err("jwt expired".into());
    }
    if claims.number("nbf")?.is_some_and(|n| n - leeway > now) {
        return Err("jwt not active yet".into());
    }
    for (name, error) in [
        ("iss", "jwt issuer mismatch"),
        ("aud", "jwt audience mismatch"),
    ] {
        if let Some(expected) = options.text(name)?
            && claims.text(name)?.unwrap_or_default() != expected
        {
            return Err(error.into());
        }
    }
    for field in Claim::ALL {
        if !claims.index.contains_key(field.name()) {
            claims.put(field.name(), Value::Null)?;
        }
    }
    claims.stringify()
}

/// Native registry entry; the compiler specializes the public typed module calls.
pub fn register(hosts: &mut crate::Hosts) -> Result<()> {
    register_with_clock(
        hosts,
        std::rc::Rc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as f64)
                .map_err(|e| e.to_string())
        }),
    )
}
pub fn register_with_clock(
    hosts: &mut crate::Hosts,
    clock: std::rc::Rc<dyn Fn() -> Result<f64>>,
) -> Result<()> {
    use crate::{HostOp, HostReply, HostType, HostValue};
    for (name, verify_call) in [(SIGN_HOST, false), (VERIFY_HOST, true)] {
        let clock = clock.clone();
        hosts.register(
            HostOp::new(
                name,
                vec![HostType::String, HostType::Bytes, HostType::String],
                HostType::String,
                false,
                move |args| {
                    let [
                        HostValue::String(input),
                        HostValue::Bytes(key),
                        HostValue::String(options),
                    ] = args.as_slice()
                    else {
                        return HostReply::Ready(Err("invalid checked JWT arguments".into()));
                    };
                    HostReply::Ready(
                        clock()
                            .and_then(|now| {
                                if verify_call {
                                    verify(input, key, options, now)
                                } else {
                                    sign(input, key, options, now)
                                }
                            })
                            .map(HostValue::String),
                    )
                },
            )
            .with_result_channel(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn random_key() -> Vec<u8> {
        let fill: crypto::Entropy = std::rc::Rc::new(crypto::fill_random);
        crypto::random_bytes(32., &fill).unwrap()
    }
    const PAYLOAD: &str = r#"{"sub":"1234567890","name":"John Doe","iat":1516239022}"#;
    const VECTOR: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.XbPfbIHMI6arZ3Y922BhjWgQzWXcXNrz0ogtVhfEd2o";
    #[test]
    fn hs256_vector_preserves_custom_claim_order_and_hydrates_missing_options() {
        assert_eq!(sign(PAYLOAD, b"secret", "{}", 1516239022.).unwrap(), VECTOR);
        let verified = verify(VECTOR, b"secret", "{}", 1516239022.).unwrap();
        let claims: Value = serde_json::from_str(&verified).unwrap();
        assert_eq!(claims["name"], "John Doe");
        assert_eq!(claims["exp"], Value::Null);
        assert_eq!(claims["sub"], "1234567890");
        let wrong_key = random_key();
        assert_eq!(
            verify(VECTOR, &wrong_key, "{}", 1516239022.).unwrap_err(),
            "invalid jwt signature"
        );
    }
    #[test]
    fn overrides_and_custom_nested_order_keep_original_positions() {
        let key = random_key();
        let token = sign(
            r#"{"sub":"old","extra":{"z":1,"a":2},"iat":8}"#,
            &key,
            r#"{"sub":"new","iss":"issuer","exp_in":20,"exp":25,"alg":"hs256"}"#,
            10.,
        )
        .unwrap();
        let payload = String::from_utf8(decode(token.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!(
            payload,
            r#"{"sub":"new","extra":{"z":1,"a":2},"iat":8,"iss":"issuer","exp":25}"#
        );
        assert_eq!(
            sign("{}", &key, r#"{"alg":"none"}"#, 10.).unwrap_err(),
            "unsupported jwt alg"
        );
        assert_eq!(
            sign(r#"{"exp":"tomorrow"}"#, &key, "{}", 10.).unwrap_err(),
            "failed to encode jwt payload"
        );
    }
    #[test]
    fn temporal_boundaries_leeway_and_expected_claims_use_explicit_clock() {
        let key = random_key();
        let token = sign(
            r#"{"iat":10,"exp":20,"nbf":10,"iss":"a","aud":"b"}"#,
            &key,
            "{}",
            10.,
        )
        .unwrap();
        for now in [10., 20.] {
            assert!(verify(&token, &key, r#"{"iss":"a","aud":"b"}"#, now).is_ok());
        }
        assert_eq!(verify(&token, &key, "{}", 21.).unwrap_err(), "jwt expired");
        assert_eq!(
            verify(&token, &key, "{}", 9.).unwrap_err(),
            "jwt issued in future"
        );
        assert!(verify(&token, &key, r#"{"leeway":1}"#, 9.).is_ok());
        assert!(verify(&token, &key, r#"{"leeway":1}"#, 21.).is_ok());
        assert_eq!(
            verify(&token, &key, r#"{"iss":"other"}"#, 15.).unwrap_err(),
            "jwt issuer mismatch"
        );
        assert_eq!(
            verify(&token, &key, r#"{"aud":"other"}"#, 15.).unwrap_err(),
            "jwt audience mismatch"
        );
        let token = sign(r#"{"iat":0,"nbf":20}"#, &key, "{}", 10.).unwrap();
        assert_eq!(
            verify(&token, &key, "{}", 19.).unwrap_err(),
            "jwt not active yet"
        );
    }
    #[test]
    fn malformed_tokens_and_large_inputs_fail_as_errors() {
        let key = random_key();
        for token in [
            "", "a.b", "a.b.c.d", ".b.c", "a..c", "a.b.", "a+b.c.d", "A.A.A",
        ] {
            assert!(verify(token, &key, "{}", 0.).is_err());
        }
        let header = b64url(br#"{"alg":"none"}"#);
        assert_eq!(
            verify(&format!("{header}.e30.AAAA"), &key, "{}", 0.).unwrap_err(),
            "unsupported jwt alg"
        );
        assert!(sign("{}", &key, &" ".repeat(MAX_INPUT + 1), 0.).is_err());
        assert!(verify(&".".repeat(MAX_TOKEN + 1), &key, "{}", 0.).is_err());
        assert!(Object::parse(r#"{"iat":1e999}"#).is_err());
    }
}
